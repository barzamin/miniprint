use std::time::Duration;

use anyhow::{anyhow, Context};
use btleplug::{
    api::{Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter, WriteType},
    platform::{Adapter, Manager, Peripheral},
};
use clap::Parser;
use futures::stream::StreamExt;
use image::{
    imageops::FilterType::{self, Gaussian},
    io::Reader as ImageReader,
};
use log::{debug, info};
use v5g::PrintMode;

use crate::v5g::{CmdPacket, CommandId};

mod v5g;

async fn locate_device(central: &Adapter, search_name: &str) -> anyhow::Result<Option<Peripheral>> {
    let mut events = central.events().await?;
    central.start_scan(ScanFilter::default()).await?;
    info!("--scanning for {}--", search_name);
    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDiscovered(id) => {
                debug!("DeviceDiscovered {:?}", id);
                let p = central.peripheral(&id).await?;
                let props = p.properties().await?.unwrap();
                info!(" = {:?}", props.local_name);
                if props.local_name.iter().any(|n| n.contains(search_name)) {
                    return Ok(Some(p));
                }
            }
            CentralEvent::DeviceUpdated(id) => debug!("DeviceUpdated {:?}", id),
            CentralEvent::DeviceConnected(id) => debug!("DeviceConnected {:?}", id),
            CentralEvent::DeviceDisconnected(id) => {
                debug!("DeviceDisconnected {:?}", id)
            }
            CentralEvent::ManufacturerDataAdvertisement {
                id,
                manufacturer_data,
            } => debug!(
                "ManufacturerDataAdvertisement {:?}: {:?}",
                id, manufacturer_data
            ),
            CentralEvent::ServiceDataAdvertisement { id, service_data } => {
                debug!("ServiceDataAdvertisement {:?}: {:?}", id, service_data)
            }
            CentralEvent::ServicesAdvertisement { id, services } => {
                debug!("ServicesAdvertisement {:?}: {:?}", id, services)
            }
        }
    }

    Ok(None)
}

#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    search_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let args = Args::parse();

    let manager = Manager::new().await.context("can't get a BLE manager")?;

    let central = manager
        .adapters()
        .await
        .context("error grabbing adapter list")?
        .into_iter()
        .nth(0)
        .ok_or(anyhow!("no BLE adapters present!"))?;

    let peripheral = locate_device(&central, &args.search_name)
        .await?
        .ok_or(anyhow!(
            "couldn't find a device with name `{}`",
            args.search_name
        ))?;

    info!("connecting to device");
    peripheral
        .connect()
        .await
        .context("BLE connection to printer failed")?;
    info!("discovering services and characteristics...");
    peripheral.discover_services().await?;

    let characteristics = peripheral.characteristics();
    // info!("  found characteristics: {:#?}", characteristics);
    let char_cmd_no_resp = characteristics
        .iter()
        .find(|c| c.uuid == v5g::CHAR_UUID_WRITE_NO_RESP)
        .ok_or(anyhow!("couldn't find WRITE_NO_RESP characteristic"))?;
    info!("found char_cmd_no_resp = {:?}", char_cmd_no_resp);

    let char_notify = characteristics
        .iter()
        .find(|c| c.uuid == v5g::CHAR_UUID_NOTIFY)
        .ok_or(anyhow!("couldn't find NOTIFY characteristic"))?;
    info!("found char_notify = {:?}", char_notify);

    peripheral.subscribe(&char_notify).await?;
    let mut notify_stream = peripheral.notifications().await?;
    tokio::spawn(async move {
        while let Some(dat) = notify_stream.next().await {
            info!(
                "NOTIFY [{:?}]: {:?} => {:?}",
                dat.uuid,
                dat.value,
                v5g::NotifyResponse::parse(&dat.value)
            );
        }
    });

    let img = ImageReader::open("epicbwbayer.png")?.decode()?;
    let img = img.resize(v5g::HORIZ_RESOLUTION, u32::MAX, FilterType::Gaussian);
    let img = img.grayscale().to_luma8();

    let printbuf = {
        let mut cmds = vec![];

        cmds.push(CmdPacket::quality(5));
        cmds.push(CmdPacket::lattice_start());

        // routine eachLinePixToCmdB
        cmds.push(CmdPacket::energy(10000));
        cmds.push(CmdPacket::print_mode(PrintMode::Image));
        cmds.push(CmdPacket::print_speed(10));

        // for _ in 0..64 {
        //     cmds.push(CmdPacket::new(CommandId::BitmapData, vec![0xff; 48]));
        // }

        for j in 0..img.height() {
            let mut row_buf = [0u8; v5g::HORIZ_RESOLUTION as usize / 8];
            for i in 0..img.width() {
                row_buf[(i as usize) / 8] >>= 1;
                // 1 = burn this dot
                row_buf[(i as usize) / 8] |= if img.get_pixel(i, j).0[0] < 127 { 0b10000000 } else { 0 };
            }
            cmds.push(CmdPacket::new(CommandId::BitmapData, row_buf.to_vec()));
        }

        // end eachLinePixToCmdB

        cmds.push(CmdPacket::new(CommandId::Paper, vec![0x30, 0x00]));
        cmds.push(CmdPacket::new(CommandId::Paper, vec![0x30, 0x00]));
        cmds.push(CmdPacket::lattice_end());

        cmds.push(CmdPacket::new(CommandId::GetDeviceState, vec![0x0])); // this triggers NOTIFY with the device state :)

        cmds
    };

    let mut cmdbuf = Vec::<u8>::new();
    for pkt in printbuf.into_iter() {
        cmdbuf.append(&mut pkt.to_vec()?);
    }

    for dat in cmdbuf.chunks(v5g::TX_SIZE) {
        debug!("CMD {:?}", dat);
        peripheral
            .write(
                &char_cmd_no_resp,
                dat,
                WriteType::WithoutResponse,
            )
            .await?;

        tokio::time::sleep(Duration::from_secs_f32(0.01)).await;
    }

    Ok(())
}
