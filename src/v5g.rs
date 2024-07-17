use std::{fmt, time::Duration};

use btleplug::api::{bleuuid::uuid_from_u16, Characteristic, Peripheral as _, WriteType};
use log::debug;
use uuid::Uuid;

use crate::printer::PrintDriver;

pub const CHAR_UUID_WRITE_NO_RESP: Uuid = uuid_from_u16(0xae01);
pub const CHAR_UUID_NOTIFY: Uuid = uuid_from_u16(0xae02);

const CRC_TABLE: [u8; 256] = [
    0x00, 0x07, 0x0e, 0x09, 0x1c, 0x1b, 0x12, 0x15, 0x38, 0x3f, 0x36, 0x31, 0x24, 0x23, 0x2a, 0x2d,
    0x70, 0x77, 0x7e, 0x79, 0x6c, 0x6b, 0x62, 0x65, 0x48, 0x4f, 0x46, 0x41, 0x54, 0x53, 0x5a, 0x5d,
    0xe0, 0xe7, 0xee, 0xe9, 0xfc, 0xfb, 0xf2, 0xf5, 0xd8, 0xdf, 0xd6, 0xd1, 0xc4, 0xc3, 0xca, 0xcd,
    0x90, 0x97, 0x9e, 0x99, 0x8c, 0x8b, 0x82, 0x85, 0xa8, 0xaf, 0xa6, 0xa1, 0xb4, 0xb3, 0xba, 0xbd,
    0xc7, 0xc0, 0xc9, 0xce, 0xdb, 0xdc, 0xd5, 0xd2, 0xff, 0xf8, 0xf1, 0xf6, 0xe3, 0xe4, 0xed, 0xea,
    0xb7, 0xb0, 0xb9, 0xbe, 0xab, 0xac, 0xa5, 0xa2, 0x8f, 0x88, 0x81, 0x86, 0x93, 0x94, 0x9d, 0x9a,
    0x27, 0x20, 0x29, 0x2e, 0x3b, 0x3c, 0x35, 0x32, 0x1f, 0x18, 0x11, 0x16, 0x03, 0x04, 0x0d, 0x0a,
    0x57, 0x50, 0x59, 0x5e, 0x4b, 0x4c, 0x45, 0x42, 0x6f, 0x68, 0x61, 0x66, 0x73, 0x74, 0x7d, 0x7a,
    0x89, 0x8e, 0x87, 0x80, 0x95, 0x92, 0x9b, 0x9c, 0xb1, 0xb6, 0xbf, 0xb8, 0xad, 0xaa, 0xa3, 0xa4,
    0xf9, 0xfe, 0xf7, 0xf0, 0xe5, 0xe2, 0xeb, 0xec, 0xc1, 0xc6, 0xcf, 0xc8, 0xdd, 0xda, 0xd3, 0xd4,
    0x69, 0x6e, 0x67, 0x60, 0x75, 0x72, 0x7b, 0x7c, 0x51, 0x56, 0x5f, 0x58, 0x4d, 0x4a, 0x43, 0x44,
    0x19, 0x1e, 0x17, 0x10, 0x05, 0x02, 0x0b, 0x0c, 0x21, 0x26, 0x2f, 0x28, 0x3d, 0x3a, 0x33, 0x34,
    0x4e, 0x49, 0x40, 0x47, 0x52, 0x55, 0x5c, 0x5b, 0x76, 0x71, 0x78, 0x7f, 0x6a, 0x6d, 0x64, 0x63,
    0x3e, 0x39, 0x30, 0x37, 0x22, 0x25, 0x2c, 0x2b, 0x06, 0x01, 0x08, 0x0f, 0x1a, 0x1d, 0x14, 0x13,
    0xae, 0xa9, 0xa0, 0xa7, 0xb2, 0xb5, 0xbc, 0xbb, 0x96, 0x91, 0x98, 0x9f, 0x8a, 0x8d, 0x84, 0x83,
    0xde, 0xd9, 0xd0, 0xd7, 0xc2, 0xc5, 0xcc, 0xcb, 0xe6, 0xe1, 0xe8, 0xef, 0xfa, 0xfd, 0xf4, 0xf3,
];

pub const TX_SIZE: usize = 60; // idk why

pub const HORIZ_RESOLUTION: u32 = 384; // 48 * 8; 48 byte data packet

fn crc8(arr: &[u8]) -> u8 {
    let mut crc = 0;
    for x in arr {
        crc = CRC_TABLE[(crc ^ x) as usize];
    }

    crc
}

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[repr(u8)]
pub enum CommandId {
    Paper = 0xa1,
    GetDeviceState = 0xa3,
    Quality = 0xa4,
    Energy = 0xaf,
    Lattice = 0xa6,
    PrintSpeed = 0xbd,
    PrintMode = 0xbe,
    BitmapData = 0xa2,
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum PrintMode {
    Image = 0,
    Text = 1,
}

#[derive(Debug)]
pub struct CmdPacket {
    pub id: u8,
    pub data: Vec<u8>,
}

impl CmdPacket {
    pub const fn new(id: CommandId, data: Vec<u8>) -> Self {
        Self { id: id as u8, data }
    }

    pub fn to_vec(mut self) -> anyhow::Result<Vec<u8>> {
        // header is 6 bytes, footer is 2
        let mut buf = Vec::with_capacity(self.data.len() + 8);
        let crc = crc8(&self.data);
        let dlen = self.data.len();

        // header
        buf.push(0x51); // magic
        buf.push(0x78);
        // cmd id, data len
        buf.push(self.id as u8);
        buf.push(0x00); // direction 0 - send to printer
        buf.push((dlen & 0xff) as u8);
        buf.push(((dlen >> 8) & 0xff) as u8);

        // data buf
        buf.append(&mut self.data);

        // footer
        buf.push(crc);
        buf.push(0xff);

        Ok(buf)
    }

    pub fn quality(level: u8) -> Self {
        assert!(level >= 1 && level <= 5, "quality levels are in [1, 5]");

        Self::new(CommandId::Quality, vec![0x31 + level])
    }

    pub fn energy(energy: u16) -> Self {
        // idk if there's some range to assert here?
        Self::new(
            CommandId::Energy,
            vec![(energy & 0xff) as u8, ((energy >> 8) & 0xff) as u8],
        )
    }

    pub fn print_speed(speed: u8) -> Self {
        Self::new(CommandId::PrintSpeed, vec![speed])
    }

    pub fn print_mode(mode: PrintMode) -> Self {
        Self::new(CommandId::PrintMode, vec![mode as u8])
    }

    pub fn lattice_start() -> Self {
        Self::new(
            CommandId::Lattice,
            vec![
                0xaa, 0x55, 0x17, 0x38, 0x44, 0x5f, 0x5f, 0x5f, 0x44, 0x38, 0x2c,
            ],
        )
    }

    pub fn lattice_end() -> Self {
        Self::new(
            CommandId::Lattice,
            vec![
                0xaa, 0x55, 0x17, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17,
            ],
        )
    }
}

#[derive(Debug)]
pub enum ParseError {
    BadMagic,
    BadTerminator,
    Checksum,
    UnknownType,
    InvalidLength,
    BadDirection(u8),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "bad magic in packet header"),
            Self::BadTerminator => write!(f, "packet should be terminated with 0xff"),
            Self::Checksum => write!(f, "packet checksum doesn't match"),
            Self::UnknownType => write!(f, "unknown packet type"),
            Self::InvalidLength => write!(f, "packet length asks us to overread"),
            Self::BadDirection(dir) => write!(
                f,
                "expected packet direction 1 (NOTIFY), but instead got {}",
                dir
            ),
        }
    }
}

impl std::error::Error for ParseError {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        self.source()
    }
}

#[derive(Debug)]
pub enum NotifyResponse {
    DeviceState(Vec<u8>),
}

impl NotifyResponse {
    pub fn parse(buf: &[u8]) -> Result<Self, ParseError> {
        let mut i = 0usize;

        if buf[0] != 0x51 || buf[1] != 0x78 {
            return Err(ParseError::BadMagic);
        }
        i += 2;

        let id = buf[i];
        i += 1;

        let pktdir = buf[i];
        i += 1;
        if pktdir != 1 {
            return Err(ParseError::BadDirection(pktdir));
        }

        // dlen: u16 = lo hi
        let dlen = buf[i] as u16 | ((buf[i + 1] as u16) << 8);
        i += 2;

        // -2 for footer
        if i + dlen as usize > buf.len() - 2 {
            return Err(ParseError::InvalidLength);
        }

        let data = buf[i..i + dlen as usize].to_vec();

        i += dlen as usize;
        let promised_crc = buf[i];
        let crc = crc8(&data);
        if promised_crc != crc {
            return Err(ParseError::Checksum);
        }

        i += 1;
        if buf[i] != 0xff {
            return Err(ParseError::BadTerminator);
        }

        match id {
            0xa3u8 => Ok(NotifyResponse::DeviceState(data)),
            _ => Err(ParseError::UnknownType),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrintSettings {
    energy: u16,
    print_mode: PrintMode,
    print_speed: u8,
    /// feed after? 0 - no feed
    feeds_after: usize,
    quality: u8,
}

impl Default for PrintSettings {
    fn default() -> Self {
        Self {
            energy: 10_000,
            print_mode: PrintMode::Image,
            print_speed: 10,
            feeds_after: 2,
            quality: 5,
        }
    }
}

#[derive(Debug)]
pub struct Driver<'a> {
    pub peripheral: &'a btleplug::platform::Peripheral,
    pub char_cmd_no_resp: &'a Characteristic,
    pub char_notify: &'a Characteristic,
}

impl<'a> PrintDriver for Driver<'a> {
    type DeviceSettings = PrintSettings;
    type Error = anyhow::Error;

    async fn print<I>(&self, img: I, settings: Self::DeviceSettings) -> Result<(), Self::Error>
    where
        I: image::GenericImageView<Pixel = image::Luma<u8>>,
    {
        let pkts = {
            let mut cmds: Vec<CmdPacket> = vec![];

            cmds.push(CmdPacket::quality(5));
            cmds.push(CmdPacket::lattice_start());

            // routine eachLinePixToCmdB
            cmds.push(CmdPacket::energy(10000));
            cmds.push(CmdPacket::print_mode(PrintMode::Image));
            cmds.push(CmdPacket::print_speed(10));

            for j in 0..img.height() {
                let mut row_buf = [0u8; HORIZ_RESOLUTION as usize / 8];
                for i in 0..img.width() {
                    row_buf[(i as usize) / 8] >>= 1;
                    // 1 = burn this dot
                    row_buf[(i as usize) / 8] |= if img.get_pixel(i, j).0[0] < 127 {
                        0b10000000
                    } else {
                        0
                    };
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

        let mut buf = Vec::<u8>::new();
        for pkt in pkts.into_iter() {
            buf.append(&mut pkt.to_vec()?);
        }

        for dat in buf.chunks(TX_SIZE) {
            debug!("CMD {:?}", dat);
            self.peripheral
                .write(self.char_cmd_no_resp, dat, WriteType::WithoutResponse)
                .await?;

            tokio::time::sleep(Duration::from_secs_f32(0.01)).await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_notify() {
        let buf = vec![
            0x51, 0x78, 0xa3, 0x01, 0x03, 0x00, 0x00, 0x01, 0x5f, 0x8f, 0xff,
        ];

        let pkt = NotifyResponse::parse(&buf).unwrap();
        println!("{:#x?}", pkt);
    }

    #[test]
    fn test_cmd_paper() {
        let pkt = CmdPacket::new(CommandId::Paper, vec![0x30, 0x00]);
        assert_eq!(
            pkt.to_vec().unwrap(),
            vec![0x51, 0x78, 0xa1, 0x00, 0x02, 0x00, 0x30, 0x00, 0xf9, 0xff]
        );

        let pkt = CmdPacket::new(CommandId::Paper, vec![0x48, 0x00]);
        assert_eq!(
            pkt.to_vec().unwrap(),
            vec![0x51, 0x78, 0xa1, 0x00, 0x02, 0x00, 0x48, 0x00, 0xf3, 0xff]
        );
    }

    #[test]
    fn test_print_mode() {
        let pkt = CmdPacket::print_mode(PrintMode::Image);
        assert_eq!(
            pkt.to_vec().unwrap(),
            vec![0x51, 0x78, 0xbe, 0x00, 0x01, 0x00, 0x00, 0x00, 0xff]
        );

        let pkt = CmdPacket::print_mode(PrintMode::Text);
        assert_eq!(
            pkt.to_vec().unwrap(),
            vec![0x51, 0x78, 0xbe, 0x00, 0x01, 0x00, 0x01, 0x07, 0xff]
        );
    }
}
