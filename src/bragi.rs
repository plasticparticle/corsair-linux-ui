use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const PACKET_SIZE: usize = 64;
const MODE_PROPERTY: u8 = 0x03;
const MODE_HARDWARE: u8 = 0x01;
const MODE_SOFTWARE: u8 = 0x02;
const BRIGHTNESS_PROPERTY: u8 = 0x02;
const BRIGHTNESS_COARSE_PROPERTY: u8 = 0x44;
const DPI_X_PROPERTY: u8 = 0x21;
const DPI_Y_PROPERTY: u8 = 0x22;
const LIGHTING_HANDLE: u8 = 0x00;
const LIGHTING_RESOURCE: u16 = 0x0001;
pub const RGB_LED_COUNT: usize = 6;
const IRONCLAW_SE_ID: &str = "HID_ID=0003:00001B1C:00002B32";
const COMMAND_DESCRIPTOR: &[u8] = &[0x06, 0x42, 0xff, 0x09, 0x01];
const INPUT_DESCRIPTOR: &[u8] = &[0x06, 0x42, 0xff, 0x09, 0x02];

#[derive(Debug)]
pub struct BragiControl {
    command_path: PathBuf,
    input_path: PathBuf,
    software_mode: bool,
    lighting_handle_open: bool,
}

impl BragiControl {
    pub fn activate() -> io::Result<Self> {
        let (command_path, input_path) =
            discover_at(Path::new("/sys/class/hidraw"))?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Wireless SE Bragi interfaces not found",
                )
            })?;
        let mut control = Self {
            command_path,
            input_path,
            software_mode: false,
            lighting_handle_open: false,
        };
        let current = control.get_mode()?;
        if current != MODE_HARDWARE && current != MODE_SOFTWARE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected Bragi mode {current}"),
            ));
        }
        if current != MODE_SOFTWARE {
            control.set_mode(MODE_SOFTWARE)?;
        }
        control.software_mode = true;
        Ok(control)
    }

    pub fn input_path(&self) -> &Path {
        &self.input_path
    }

    fn open_command(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.command_path)
    }

    fn exchange(&self, packet: [u8; PACKET_SIZE]) -> io::Result<[u8; PACKET_SIZE]> {
        let mut device = self.open_command()?;
        let mut report = [0_u8; PACKET_SIZE + 1];
        report[1..].copy_from_slice(&packet);
        device.write_all(&report)?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut response = [0_u8; PACKET_SIZE];
        loop {
            match device.read(&mut response) {
                Ok(PACKET_SIZE) => break,
                Ok(size) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("short Bragi response: {size} bytes"),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "timed out waiting for Bragi response",
                        ));
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        if response[1] != packet[1] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Bragi response command mismatch: sent 0x{:02x}, received 0x{:02x}",
                    packet[1], response[1]
                ),
            ));
        }
        Ok(response)
    }

    fn transact(&self, packet: [u8; PACKET_SIZE]) -> io::Result<[u8; PACKET_SIZE]> {
        let response = self.exchange(packet)?;
        if response[2] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Bragi command rejected with error 0x{:02x}", response[2]),
            ));
        }
        Ok(response)
    }

    fn get_mode(&self) -> io::Result<u8> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..4].copy_from_slice(&[0x08, 0x02, MODE_PROPERTY, 0x00]);
        Ok(self.transact(packet)?[3])
    }

    fn set_mode(&self, mode: u8) -> io::Result<()> {
        self.set_property(MODE_PROPERTY, u16::from(mode))
    }

    fn set_property(&self, property: u8, value: u16) -> io::Result<()> {
        self.transact(property_packet(property, value)).map(|_| ())
    }

    fn get_property(&self, property: u8) -> io::Result<u32> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..4].copy_from_slice(&[0x08, 0x02, property, 0x00]);
        let response = self.transact(packet)?;
        Ok(u32::from(response[3]) | (u32::from(response[4]) << 8) | (u32::from(response[5]) << 16))
    }

    fn close_lighting_handle(&mut self) -> io::Result<()> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..5].copy_from_slice(&[0x08, 0x05, 0x01, LIGHTING_HANDLE, 0x00]);
        let result = self.transact(packet).map(|_| ());
        self.lighting_handle_open = false;
        result
    }

    fn open_lighting_handle(&mut self) -> io::Result<()> {
        if self.lighting_handle_open {
            return Ok(());
        }
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..6].copy_from_slice(&[
            0x08,
            0x0d,
            LIGHTING_HANDLE,
            LIGHTING_RESOURCE as u8,
            (LIGHTING_RESOURCE >> 8) as u8,
            0x00,
        ]);
        let mut response = self.exchange(packet)?;
        if response[2] == 0x03 {
            let _ = self.close_lighting_handle();
            response = self.exchange(packet)?;
        }
        if response[2] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "Bragi lighting resource rejected with error 0x{:02x}",
                    response[2]
                ),
            ));
        }
        self.lighting_handle_open = true;
        Ok(())
    }

    fn lighting_resource_size(&self) -> io::Result<Option<usize>> {
        let mut packet = [0_u8; PACKET_SIZE];
        packet[..4].copy_from_slice(&[0x08, 0x09, LIGHTING_HANDLE, 0x00]);
        let response = self.exchange(packet)?;
        if matches!(response[2], 0x01 | 0x05) {
            // Some Ironclaw firmware opens and writes this resource but does
            // not implement the optional handle-size probe.
            return Ok(None);
        }
        if response[2] != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Bragi lighting size probe failed with error 0x{:02x}",
                    response[2]
                ),
            ));
        }
        let size = u32::from_le_bytes([response[5], response[6], response[7], response[8]]);
        usize::try_from(size)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Bragi lighting size is too large",
                )
            })
            .map(Some)
    }

    pub fn enable_rgb(&mut self) -> io::Result<usize> {
        self.open_lighting_handle()?;
        let expected_size = RGB_LED_COUNT * 3;
        if let Some(size) = self.lighting_resource_size()? {
            if size != expected_size {
                let _ = self.close_lighting_handle();
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "unexpected Bragi lighting resource size: {size} bytes (expected {expected_size})"
                    ),
                ));
            }
        }

        // Older Ironclaw firmware exposes fine brightness; some related Bragi
        // devices expose only the coarse property. Payload colors are scaled
        // in software, so this property stays at its maximum while active.
        if self.get_property(BRIGHTNESS_PROPERTY).is_ok() {
            self.set_property(BRIGHTNESS_PROPERTY, 1000)?;
        } else if self.get_property(BRIGHTNESS_COARSE_PROPERTY).is_ok() {
            self.set_property(BRIGHTNESS_COARSE_PROPERTY, 3)?;
        }
        Ok(RGB_LED_COUNT)
    }

    pub fn write_rgb(&mut self, colors: &[[u8; 3]; RGB_LED_COUNT]) -> io::Result<()> {
        self.open_lighting_handle()?;
        let packet = rgb_packet(colors);
        self.transact(packet).map(|_| ())
    }

    /// Apply the same live resolution to both sensor axes.
    ///
    /// Bragi exposes current X and Y resolution as properties 0x21 and 0x22.
    /// The Ironclaw accepts DPI directly as a little-endian 16-bit value while
    /// it is in software mode.
    pub fn set_dpi(&self, dpi: u32) -> io::Result<()> {
        let dpi = u16::try_from(dpi).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "DPI exceeds the Bragi value range",
            )
        })?;
        if !(100..=26_000).contains(&dpi) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "DPI must be between 100 and 26000",
            ));
        }
        self.set_property(DPI_X_PROPERTY, dpi)?;
        self.set_property(DPI_Y_PROPERTY, dpi)
    }
}

impl Drop for BragiControl {
    fn drop(&mut self) {
        if self.lighting_handle_open {
            let _ = self.close_lighting_handle();
        }
        if self.software_mode {
            let _ = self.set_mode(MODE_HARDWARE);
            self.software_mode = false;
        }
    }
}

fn rgb_packet(colors: &[[u8; 3]; RGB_LED_COUNT]) -> [u8; PACKET_SIZE] {
    let mut packet = [0_u8; PACKET_SIZE];
    let data_len = (RGB_LED_COUNT * 3) as u32;
    packet[..7].copy_from_slice(&[
        0x08,
        0x06,
        LIGHTING_HANDLE,
        data_len as u8,
        (data_len >> 8) as u8,
        (data_len >> 16) as u8,
        (data_len >> 24) as u8,
    ]);
    for (index, color) in colors.iter().enumerate() {
        packet[7 + index] = color[0];
        packet[7 + RGB_LED_COUNT + index] = color[1];
        packet[7 + RGB_LED_COUNT * 2 + index] = color[2];
    }
    packet
}

fn property_packet(property: u8, value: u16) -> [u8; PACKET_SIZE] {
    let mut packet = [0_u8; PACKET_SIZE];
    packet[..6].copy_from_slice(&[0x08, 0x01, property, 0x00, value as u8, (value >> 8) as u8]);
    packet
}

fn discover_at(root: &Path) -> io::Result<Option<(PathBuf, PathBuf)>> {
    let mut command = None;
    let mut input = None;
    for entry in fs::read_dir(root)?.flatten() {
        let device = entry.path().join("device");
        let uevent = fs::read_to_string(device.join("uevent")).unwrap_or_default();
        if !uevent.lines().any(|line| line == IRONCLAW_SE_ID) {
            continue;
        }
        let descriptor = fs::read(device.join("report_descriptor")).unwrap_or_default();
        let node = PathBuf::from("/dev").join(entry.file_name());
        if descriptor.starts_with(COMMAND_DESCRIPTOR) {
            command = Some(node);
        } else if descriptor.starts_with(INPUT_DESCRIPTOR) {
            input = Some(node);
        }
    }
    Ok(command.zip(input))
}

pub fn button_mask(report: &[u8]) -> Option<u16> {
    if report.len() == PACKET_SIZE && report[1] == 0x02 {
        Some(u16::from_le_bytes([report[2], report[3]]))
    } else {
        None
    }
}

pub fn source_for_bit(bit: u8) -> String {
    format!("BRAGI_BUTTON:{}", bit + 1)
}

pub fn linux_code_for_bit(bit: u8) -> u16 {
    const LUT: [u16; 16] = [
        272, 273, 274, 276, 275, 277, 278, 280, 281, 279, 282, 283, 284, 285, 286, 287,
    ];
    LUT[usize::from(bit.min(15))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_extended_button_mask() {
        let mut report = [0_u8; PACKET_SIZE];
        report[1] = 0x02;
        report[2] = 0x80;
        report[3] = 0x02;
        assert_eq!(button_mask(&report), Some(0x0280));
        assert_eq!(source_for_bit(7), "BRAGI_BUTTON:8");
        assert_eq!(linux_code_for_bit(3), 276);
        assert_eq!(linux_code_for_bit(9), 279);
    }

    #[test]
    fn creates_planar_six_led_rgb_packet() {
        let colors = [
            [0x10, 0x11, 0x12],
            [0x20, 0x21, 0x22],
            [0x30, 0x31, 0x32],
            [0x40, 0x41, 0x42],
            [0x50, 0x51, 0x52],
            [0x60, 0x61, 0x62],
        ];
        let packet = rgb_packet(&colors);
        assert_eq!(&packet[..7], &[0x08, 0x06, 0x00, 18, 0, 0, 0]);
        assert_eq!(&packet[7..13], &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
        assert_eq!(&packet[13..19], &[0x11, 0x21, 0x31, 0x41, 0x51, 0x61]);
        assert_eq!(&packet[19..25], &[0x12, 0x22, 0x32, 0x42, 0x52, 0x62]);
    }

    #[test]
    fn creates_little_endian_dpi_property_packets() {
        let x = property_packet(DPI_X_PROPERTY, 3_200);
        let y = property_packet(DPI_Y_PROPERTY, 3_200);
        assert_eq!(&x[..6], &[0x08, 0x01, 0x21, 0x00, 0x80, 0x0c]);
        assert_eq!(&y[..6], &[0x08, 0x01, 0x22, 0x00, 0x80, 0x0c]);
    }

    #[test]
    #[ignore = "requires a connected 1b1c:2b32 mouse and briefly changes its lighting"]
    fn probes_and_writes_connected_ironclaw_rgb() {
        let mut control = BragiControl::activate().expect("activate connected Ironclaw");
        assert_eq!(
            control.enable_rgb().expect("probe lighting resource"),
            RGB_LED_COUNT
        );
        control
            .write_rgb(&[[0x08, 0x30, 0x2a]; RGB_LED_COUNT])
            .expect("write dim mint lighting frame");
        thread::sleep(Duration::from_millis(800));
    }

    #[test]
    #[ignore = "requires a connected 1b1c:2b32 mouse and briefly changes its sensitivity"]
    fn writes_connected_ironclaw_dpi() {
        let control = BragiControl::activate().expect("activate connected Ironclaw");
        control.set_dpi(800).expect("write 800 DPI to both axes");
        thread::sleep(Duration::from_millis(500));
        control
            .set_dpi(1_600)
            .expect("restore 1600 DPI to both axes");
    }
}
