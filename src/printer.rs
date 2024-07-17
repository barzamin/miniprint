use image::{GenericImageView, Luma};
use std::future::Future;

pub trait PrintDriver {
    type DeviceSettings;
    type Error; //: std::error::Error;

    fn print<I>(
        &self,
        img: I,
        settings: Self::DeviceSettings,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send
    where
        I: GenericImageView<Pixel = Luma<u8>> + Send;
}
