//! Raspberry Pi 5 board power from the PMIC ADC.
//!
//! The Raspberry Pi 5 PMIC reports the current and the voltage of each of its
//! supply rails through the VideoCore firmware command `pmic_read_adc` (the
//! one behind `vcgencmd pmic_read_adc`). Summing V × A over the rails gives a
//! measured figure for everything the board feeds through its PMIC: SoC,
//! RAM, Wi-Fi, on-board 3.3 V and 1.8 V consumers.
//!
//! It does not include what is powered directly from the 5 V input (USB
//! devices, PCIe/NVMe HATs, the fan, a display fed from the 5 V pins) nor the
//! power-supply losses: the PMIC reports the 5 V input voltage (`EXT5V_V`) but
//! not its current. The reading is therefore a measured lower bound of the
//! board's consumption.
//!
//! The firmware is reached through `/dev/vcio_gencmd`, which the Raspberry Pi
//! udev rules make accessible to the `video` group. No root access and no
//! external `vcgencmd` binary are needed, so the sensor also works in the
//! Docker image once that single device is passed to the container.

use crate::model::{Component, Confidence, PowerSensor, SensorError, SensorReading};

pub const DEFAULT_GENCMD_DEVICE: &str = "/dev/vcio_gencmd";
pub const GENCMD_DEVICE_ENV: &str = "POWERWATCH_VCIO_GENCMD_PATH";
pub const PMIC_COMMAND: &str = "pmic_read_adc";

/// Total power of the rails found in a `pmic_read_adc` answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmicPower {
    pub watts: f64,
    /// Number of rails that reported both a current and a voltage.
    pub rails: usize,
}

/// Parses the text answer of `pmic_read_adc`.
///
/// Each rail appears on two lines, `<RAIL>_A current(n)=<amps>A` and
/// `<RAIL>_V volt(n)=<volts>V`. Rails that only report a voltage (`EXT5V`,
/// `BATT`) cannot contribute a power and are ignored. An answer without any
/// complete rail (for instance a firmware error on boards that do not know the
/// command) is rejected.
pub fn parse_pmic_adc(text: &str) -> Result<PmicPower, SensorError> {
    let mut currents = std::collections::BTreeMap::new();
    let mut voltages = std::collections::BTreeMap::new();

    for line in text.lines() {
        let Some((label, rest)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };

        let (rail, unit, map) = if let Some(rail) = label.strip_suffix("_A") {
            (rail, 'A', &mut currents)
        } else if let Some(rail) = label.strip_suffix("_V") {
            (rail, 'V', &mut voltages)
        } else {
            continue;
        };

        let Some(number) = value.trim().strip_suffix(unit) else {
            continue;
        };
        let number = number
            .parse::<f64>()
            .map_err(|e| SensorError::ReadFailed(format!("invalid PMIC value for {label}: {e}")))?;
        if !number.is_finite() {
            return Err(SensorError::ReadFailed(format!(
                "non-finite PMIC value for {label}"
            )));
        }

        map.insert(rail.to_string(), number);
    }

    let mut watts = 0.0;
    let mut rails = 0;
    for (rail, amps) in &currents {
        if let Some(volts) = voltages.get(rail) {
            watts += amps * volts;
            rails += 1;
        }
    }

    if rails == 0 {
        return Err(SensorError::Unavailable(
            "no PMIC rail with both current and voltage in the pmic_read_adc answer".to_string(),
        ));
    }

    Ok(PmicPower { watts, rails })
}

/// Size of the command/answer area of a gencmd mailbox message, as used by
/// the firmware tools.
pub const GENCMD_MAX_STRING: usize = 4 * 1024;
const GET_GENCMD_RESULT_TAG: u32 = 0x0003_0080;
/// Words before the command string: size, request code, tag, buffer length,
/// request length, firmware error code.
const GENCMD_HEADER_WORDS: usize = 6;
const GENCMD_STRING_WORDS: usize = GENCMD_MAX_STRING / 4;
const GENCMD_TOTAL_WORDS: usize = GENCMD_HEADER_WORDS + GENCMD_STRING_WORDS + 1;

/// Builds the mailbox property message for a gencmd command.
pub fn build_gencmd_request(command: &str) -> Result<Vec<u32>, SensorError> {
    let bytes = command.as_bytes();
    if bytes.len() + 1 >= GENCMD_MAX_STRING {
        return Err(SensorError::ReadFailed(format!(
            "gencmd command too long ({} bytes)",
            bytes.len()
        )));
    }

    let mut words = vec![0u32; GENCMD_TOTAL_WORDS];
    words[0] = (GENCMD_TOTAL_WORDS * 4) as u32; // message size in bytes
    words[1] = 0; // process request
    words[2] = GET_GENCMD_RESULT_TAG;
    words[3] = GENCMD_MAX_STRING as u32; // value buffer size
    words[4] = 0; // request length, set by the firmware to the answer length
    words[5] = 0; // firmware error code

    // The command is copied byte for byte into the value buffer, in memory
    // order; the zeroed buffer provides the terminating NUL and the end tag.
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        words[GENCMD_HEADER_WORDS + index] = u32::from_ne_bytes(word);
    }

    Ok(words)
}

/// Extracts the answer from a mailbox message filled in by the firmware.
pub fn parse_gencmd_response(words: &[u32]) -> Result<String, SensorError> {
    if words.len() < GENCMD_TOTAL_WORDS {
        return Err(SensorError::ReadFailed(
            "gencmd answer buffer is too short".to_string(),
        ));
    }

    let error = words[5];
    if error != 0 {
        return Err(SensorError::ReadFailed(format!(
            "the firmware returned gencmd error {error}"
        )));
    }

    let bytes: Vec<u8> = words[GENCMD_HEADER_WORDS..GENCMD_HEADER_WORDS + GENCMD_STRING_WORDS]
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .take_while(|byte| *byte != 0)
        .collect();

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// `_IOWR(100, 0, char *)`: the vcio "mailbox property" request. The argument
/// size is the size of a pointer, so the number differs between 64-bit and
/// 32-bit ARM.
pub fn mbox_property_request(pointer_size: usize) -> u64 {
    const READ_WRITE: u64 = 3;
    const VCIO_IOC_MAGIC: u64 = 100;
    (READ_WRITE << 30) | ((pointer_size as u64) << 16) | (VCIO_IOC_MAGIC << 8)
}

/// Runs a firmware command and returns its text answer.
pub trait GencmdSource: Send {
    fn run(&mut self, command: &str) -> Result<String, SensorError>;
}

#[cfg(target_os = "linux")]
pub use vcio::VcioGencmd;

#[cfg(target_os = "linux")]
mod vcio {
    use super::{build_gencmd_request, mbox_property_request, parse_gencmd_response, GencmdSource};
    use crate::model::SensorError;
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::path::Path;

    /// The firmware mailbox, through `/dev/vcio_gencmd`.
    pub struct VcioGencmd {
        device: File,
    }

    impl VcioGencmd {
        pub fn open(path: &Path) -> Result<Self, SensorError> {
            let device = File::open(path).map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    SensorError::Unavailable(format!("{} does not exist", path.display()))
                }
                std::io::ErrorKind::PermissionDenied => SensorError::PermissionDenied(format!(
                    "no access to {} (the user needs the video group)",
                    path.display()
                )),
                _ => SensorError::ReadFailed(format!("cannot open {}: {e}", path.display())),
            })?;

            Ok(Self { device })
        }
    }

    impl GencmdSource for VcioGencmd {
        fn run(&mut self, command: &str) -> Result<String, SensorError> {
            let mut words = build_gencmd_request(command)?;
            let request = mbox_property_request(std::mem::size_of::<*mut u8>());

            // SAFETY: `words` is a live, writable buffer of the size announced
            // in its first word; the driver copies it in and the answer back
            // into the same memory before returning.
            let result =
                unsafe { libc::ioctl(self.device.as_raw_fd(), request as _, words.as_mut_ptr()) };
            if result < 0 {
                return Err(SensorError::ReadFailed(format!(
                    "gencmd mailbox call failed: {}",
                    std::io::Error::last_os_error()
                )));
            }

            parse_gencmd_response(&words)
        }
    }
}

/// Measured board power of a Raspberry Pi 5, reported on the `cpu` sensor so
/// that the Hub, alerts and history keep working unchanged.
pub struct RpiPmicSensor<S: GencmdSource> {
    source: S,
}

impl<S: GencmdSource> RpiPmicSensor<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }
}

impl<S: GencmdSource> PowerSensor for RpiPmicSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        let answer = self.source.run(PMIC_COMMAND)?;
        let power = parse_pmic_adc(&answer)?;

        Ok(SensorReading {
            component: Component::Cpu,
            watts: power.watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two real answers captured on a Raspberry Pi 5 (Debian 13, idle kiosk).
    const PANDA_1: &str = "\
 3V7_WL_SW_A current(0)=0.00000000A
   3V3_SYS_A current(1)=0.13663020A
   1V8_SYS_A current(2)=0.25666960A
  DDR_VDD2_A current(3)=0.07124290A
  DDR_VDDQ_A current(4)=0.00000000A
   1V1_SYS_A current(5)=0.18640260A
    0V8_SW_A current(6)=0.33962360A
  VDD_CORE_A current(7)=0.79477000A
   3V3_DAC_A current(17)=0.00000000A
   3V3_ADC_A current(18)=0.00012210A
   0V8_AON_A current(16)=0.00341880A
      HDMI_A current(22)=0.01709400A
 3V7_WL_SW_V volt(8)=3.71369600V
   3V3_SYS_V volt(9)=3.31003300V
   1V8_SYS_V volt(10)=1.80317300V
  DDR_VDD2_V volt(11)=1.10805800V
  DDR_VDDQ_V volt(12)=0.60512760V
   1V1_SYS_V volt(13)=1.10476100V
    0V8_SW_V volt(14)=0.82051210V
  VDD_CORE_V volt(15)=0.77120810V
   3V3_DAC_V volt(20)=3.31318300V
   3V3_ADC_V volt(21)=3.30952000V
   0V8_AON_V volt(19)=0.79794790V
      HDMI_V volt(23)=5.12416000V
     EXT5V_V volt(24)=5.12416000V
      BATT_V volt(25)=0.00000000V
";

    const PANDA_2: &str = "\
 3V7_WL_SW_A current(0)=0.00000000A
   3V3_SYS_A current(1)=0.13175060A
   1V8_SYS_A current(2)=0.25471770A
  DDR_VDD2_A current(3)=0.01171116A
  DDR_VDDQ_A current(4)=0.00000000A
   1V1_SYS_A current(5)=0.18542670A
    0V8_SW_A current(6)=0.36792560A
  VDD_CORE_A current(7)=0.80370000A
   3V3_DAC_A current(17)=0.00000000A
   3V3_ADC_A current(18)=0.00012210A
   0V8_AON_A current(16)=0.00335775A
      HDMI_A current(22)=0.01684980A
 3V7_WL_SW_V volt(8)=3.71369600V
   3V3_SYS_V volt(9)=3.31096100V
   1V8_SYS_V volt(10)=1.81294100V
  DDR_VDD2_V volt(11)=1.10769100V
  DDR_VDDQ_V volt(12)=0.60073200V
   1V1_SYS_V volt(13)=1.10402800V
    0V8_SW_V volt(14)=0.79999920V
  VDD_CORE_V volt(15)=0.77093940V
   3V3_DAC_V volt(20)=3.31409900V
   3V3_ADC_V volt(21)=3.30952000V
   0V8_AON_V volt(19)=0.79853400V
      HDMI_V volt(23)=5.12282000V
     EXT5V_V volt(24)=5.11880000V
      BATT_V volt(25)=0.00000000V
";

    #[test]
    fn sums_every_rail_of_a_real_pi5_answer() {
        let power = parse_pmic_adc(PANDA_1).unwrap();

        assert_eq!(power.rails, 12);
        assert!(
            (power.watts - 2.182_264_594).abs() < 1e-6,
            "{}",
            power.watts
        );
    }

    #[test]
    fn sums_a_second_real_answer() {
        let power = parse_pmic_adc(PANDA_2).unwrap();

        assert_eq!(power.rails, 12);
        assert!(
            (power.watts - 2.119_045_919).abs() < 1e-6,
            "{}",
            power.watts
        );
    }

    #[test]
    fn voltage_only_rails_do_not_contribute() {
        let with_input = parse_pmic_adc(
            "VDD_CORE_A current(7)=1.0A\nVDD_CORE_V volt(15)=0.8V\nEXT5V_V volt(24)=5.1V\n",
        )
        .unwrap();

        assert_eq!(with_input.rails, 1);
        assert!((with_input.watts - 0.8).abs() < 1e-12);
    }

    #[test]
    fn rails_can_appear_in_any_order() {
        let power = parse_pmic_adc(
            "X_V volt(1)=2.0V\nY_A current(2)=3.0A\nX_A current(0)=0.5A\nY_V volt(3)=1.0V\n",
        )
        .unwrap();

        assert_eq!(power.rails, 2);
        assert!((power.watts - 4.0).abs() < 1e-12);
    }

    #[test]
    fn an_answer_without_rails_is_unavailable() {
        let error = parse_pmic_adc("error=2 error_msg=\"Command not registered\"\n").unwrap_err();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }

    #[test]
    fn an_empty_answer_is_unavailable() {
        assert!(matches!(
            parse_pmic_adc("").unwrap_err(),
            SensorError::Unavailable(_)
        ));
    }

    #[test]
    fn a_malformed_number_is_reported() {
        let error =
            parse_pmic_adc("VDD_CORE_A current(7)=abcA\nVDD_CORE_V volt(15)=0.8V\n").unwrap_err();

        assert!(matches!(error, SensorError::ReadFailed(_)));
    }

    #[test]
    fn the_request_follows_the_gencmd_mailbox_layout() {
        let words = build_gencmd_request("pmic_read_adc").unwrap();

        assert_eq!(words.len(), 6 + 1024 + 1);
        assert_eq!(words[0], (words.len() * 4) as u32);
        assert_eq!(words[1], 0);
        assert_eq!(words[2], 0x0003_0080);
        assert_eq!(words[3], 4096);
        assert_eq!(words[4], 0);
        assert_eq!(words[5], 0);
        assert_eq!(*words.last().unwrap(), 0, "end tag");

        let command: Vec<u8> = words[6..6 + 1024]
            .iter()
            .flat_map(|word| word.to_ne_bytes())
            .collect();
        assert_eq!(&command[..13], b"pmic_read_adc");
        assert_eq!(command[13], 0, "NUL terminator");
    }

    #[test]
    fn an_oversized_command_is_rejected() {
        let command = "x".repeat(GENCMD_MAX_STRING);

        assert!(build_gencmd_request(&command).is_err());
    }

    fn response_with(error: u32, text: &str) -> Vec<u32> {
        let mut words = build_gencmd_request("").unwrap();
        words[5] = error;
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(GENCMD_MAX_STRING, 0);
        for (index, chunk) in bytes.chunks(4).enumerate() {
            words[6 + index] = u32::from_ne_bytes(chunk.try_into().unwrap());
        }
        words
    }

    #[test]
    fn the_answer_is_read_up_to_the_nul_terminator() {
        let words = response_with(0, "VDD_CORE_A current(7)=0.79A\n");

        assert_eq!(
            parse_gencmd_response(&words).unwrap(),
            "VDD_CORE_A current(7)=0.79A\n"
        );
    }

    #[test]
    fn a_firmware_error_code_is_reported() {
        let words = response_with(1, "");

        assert!(matches!(
            parse_gencmd_response(&words).unwrap_err(),
            SensorError::ReadFailed(_)
        ));
    }

    #[test]
    fn a_truncated_buffer_is_rejected() {
        assert!(parse_gencmd_response(&[0u32; 8]).is_err());
    }

    #[test]
    fn mailbox_request_numbers_match_the_kernel_encoding() {
        // strace on a Raspberry Pi 5 (aarch64) shows
        // _IOC(_IOC_READ|_IOC_WRITE, 0x64, 0, 0x8) for vcgencmd.
        assert_eq!(mbox_property_request(8), 0xC008_6400);
        // 32-bit ARM passes a 4-byte pointer.
        assert_eq!(mbox_property_request(4), 0xC004_6400);
    }

    struct FakeGencmd {
        answer: Result<String, SensorError>,
        commands: Vec<String>,
    }

    impl GencmdSource for FakeGencmd {
        fn run(&mut self, command: &str) -> Result<String, SensorError> {
            self.commands.push(command.to_string());
            self.answer.clone()
        }
    }

    #[test]
    fn the_sensor_reports_a_measured_cpu_reading() {
        let mut sensor = RpiPmicSensor::new(FakeGencmd {
            answer: Ok(PANDA_1.to_string()),
            commands: Vec::new(),
        });

        let reading = sensor.sample().unwrap();

        assert_eq!(sensor.source.commands, vec!["pmic_read_adc".to_string()]);
        assert_eq!(reading.component, Component::Cpu);
        assert_eq!(reading.confidence, Confidence::Measured);
        assert!((reading.watts - 2.182_264_594).abs() < 1e-6);
    }

    #[test]
    fn source_errors_are_propagated() {
        let mut sensor = RpiPmicSensor::new(FakeGencmd {
            answer: Err(SensorError::PermissionDenied(
                "/dev/vcio_gencmd".to_string(),
            )),
            commands: Vec::new(),
        });

        assert!(matches!(
            sensor.sample().unwrap_err(),
            SensorError::PermissionDenied(_)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opening_a_missing_device_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();

        let error = VcioGencmd::open(&dir.path().join("vcio_gencmd"))
            .err()
            .unwrap();

        assert!(matches!(error, SensorError::Unavailable(_)));
    }
}
