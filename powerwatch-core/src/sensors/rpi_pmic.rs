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
use std::time::{Duration, Instant};

pub const DEFAULT_GENCMD_DEVICE: &str = "/dev/vcio_gencmd";
pub const GENCMD_DEVICE_ENV: &str = "POWERWATCH_VCIO_GENCMD_PATH";
pub const PMIC_COMMAND: &str = "pmic_read_adc";

/// Rails a Raspberry Pi 5 always reports: the SoC core and the three main
/// system supplies. An answer missing one of them is truncated or comes from
/// something that is not a Pi 5 PMIC, and must not be turned into a figure.
pub const REQUIRED_RAILS: [&str; 4] = ["VDD_CORE", "3V3_SYS", "1V8_SYS", "1V1_SYS"];
/// A Raspberry Pi 5 reports 12 rails with both a current and a voltage. A few
/// auxiliary rails may disappear with a firmware change; fewer than this is a
/// truncated answer.
pub const MIN_COMPLETE_RAILS: usize = 8;
/// No PMIC rail of the board runs above the 5 V input (with some margin).
pub const MAX_RAIL_VOLTS: f64 = 6.0;
/// Well above what any single rail can deliver.
pub const MAX_RAIL_AMPS: f64 = 10.0;
/// A running board always draws more than this through its PMIC.
pub const MIN_PLAUSIBLE_WATTS: f64 = 0.5;
/// The official supply is 27 W; the rails behind the PMIC stay below it.
pub const MAX_PLAUSIBLE_WATTS: f64 = 30.0;

/// Total power of the rails found in a `pmic_read_adc` answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmicPower {
    pub watts: f64,
    /// Number of rails that reported both a current and a voltage.
    pub rails: usize,
}

/// The rails of a `pmic_read_adc` answer, before any plausibility check.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PmicRails {
    currents: std::collections::BTreeMap<String, f64>,
    voltages: std::collections::BTreeMap<String, f64>,
}

impl PmicRails {
    /// Whether the answer contained no rail line at all (for instance the
    /// firmware error of a board that does not know the command).
    pub fn is_empty(&self) -> bool {
        self.currents.is_empty() && self.voltages.is_empty()
    }

    /// Sum of V × A over the rails that report both, without validation.
    pub fn power(&self) -> PmicPower {
        let mut watts = 0.0;
        let mut rails = 0;
        for (rail, amps) in &self.currents {
            if let Some(volts) = self.voltages.get(rail) {
                watts += amps * volts;
                rails += 1;
            }
        }
        PmicPower { watts, rails }
    }

    /// Checks that the answer is a complete and plausible Raspberry Pi 5
    /// reading, then returns its total power.
    ///
    /// A partial answer would silently produce a figure that is too low, so
    /// anything incomplete is an error rather than a smaller number.
    pub fn validated_power(&self) -> Result<PmicPower, SensorError> {
        if self.is_empty() {
            return Err(SensorError::Unavailable(
                "no PMIC rail in the pmic_read_adc answer".to_string(),
            ));
        }

        let incomplete = |detail: String| {
            SensorError::ReadFailed(format!("incomplete pmic_read_adc answer: {detail}"))
        };

        // A rail with a current but no voltage would be dropped from the sum.
        if let Some(rail) = self
            .currents
            .keys()
            .find(|rail| !self.voltages.contains_key(*rail))
        {
            return Err(incomplete(format!(
                "rail {rail} has a current but no voltage"
            )));
        }

        for rail in REQUIRED_RAILS {
            if !self.currents.contains_key(rail) {
                return Err(incomplete(format!("rail {rail} is missing")));
            }
        }

        for (rail, amps) in &self.currents {
            if !(0.0..=MAX_RAIL_AMPS).contains(amps) {
                return Err(SensorError::ReadFailed(format!(
                    "implausible PMIC current for {rail}: {amps} A"
                )));
            }
        }
        for (rail, volts) in &self.voltages {
            if !(0.0..=MAX_RAIL_VOLTS).contains(volts) {
                return Err(SensorError::ReadFailed(format!(
                    "implausible PMIC voltage for {rail}: {volts} V"
                )));
            }
        }

        let power = self.power();
        if power.rails < MIN_COMPLETE_RAILS {
            return Err(incomplete(format!(
                "{} complete rails, at least {MIN_COMPLETE_RAILS} expected",
                power.rails
            )));
        }
        if !(MIN_PLAUSIBLE_WATTS..=MAX_PLAUSIBLE_WATTS).contains(&power.watts) {
            return Err(SensorError::ReadFailed(format!(
                "implausible PMIC total: {:.3} W over {} rails",
                power.watts, power.rails
            )));
        }

        Ok(power)
    }
}

/// Parses the rails of the text answer of `pmic_read_adc`.
///
/// Each rail appears on two lines, `<RAIL>_A current(n)=<amps>A` and
/// `<RAIL>_V volt(n)=<volts>V`. Rails that only report a voltage (`EXT5V`,
/// `BATT`) cannot contribute a power. Lines that are not rail lines are
/// ignored; a rail line with an unreadable value is an error.
pub fn parse_pmic_rails(text: &str) -> Result<PmicRails, SensorError> {
    let mut rails = PmicRails::default();

    for line in text.lines() {
        let Some((label, rest)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let Some((_, value)) = rest.split_once('=') else {
            continue;
        };

        let (rail, unit, map) = if let Some(rail) = label.strip_suffix("_A") {
            (rail, 'A', &mut rails.currents)
        } else if let Some(rail) = label.strip_suffix("_V") {
            (rail, 'V', &mut rails.voltages)
        } else {
            continue;
        };

        let Some(number) = value.trim().strip_suffix(unit) else {
            return Err(SensorError::ReadFailed(format!(
                "PMIC value for {label} does not end with {unit}"
            )));
        };
        let number = number
            .parse::<f64>()
            .map_err(|e| SensorError::ReadFailed(format!("invalid PMIC value for {label}: {e}")))?;
        if !number.is_finite() {
            return Err(SensorError::ReadFailed(format!(
                "non-finite PMIC value for {label}"
            )));
        }

        if map.insert(rail.to_string(), number).is_some() {
            return Err(SensorError::ReadFailed(format!(
                "PMIC rail {label} is reported twice"
            )));
        }
    }

    Ok(rails)
}

/// Parses and validates the text answer of `pmic_read_adc`.
///
/// An answer without any rail (for instance a firmware error on boards that
/// do not know the command) is `Unavailable`; an incomplete or implausible
/// one is `ReadFailed`.
pub fn parse_pmic_adc(text: &str) -> Result<PmicPower, SensorError> {
    parse_pmic_rails(text)?.validated_power()
}

/// Whether a board model string names a board with the Raspberry Pi 5 PMIC.
pub fn is_pi5_family(model: &str) -> bool {
    [
        "Raspberry Pi 5",
        "Raspberry Pi Compute Module 5",
        "Raspberry Pi 500",
    ]
    .iter()
    .any(|name| model.contains(name))
}

/// Extracts the board model from the text of `/proc/cpuinfo`.
pub fn model_from_cpuinfo(cpuinfo: &str) -> Option<&str> {
    cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Model").then(|| value.trim())
    })
}

/// Whether this machine identifies itself as a Raspberry Pi 5 family board.
///
/// The device tree is preferred; `/proc/cpuinfo` is the fallback for
/// containers, where `/sys/firmware` is usually masked.
#[cfg(target_os = "linux")]
pub fn running_on_pi5_family() -> bool {
    if let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") {
        return is_pi5_family(&model);
    }
    std::fs::read_to_string("/proc/cpuinfo")
        .is_ok_and(|cpuinfo| model_from_cpuinfo(&cpuinfo).is_some_and(is_pi5_family))
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

/// Delay between two attempts while the PMIC has never answered.
pub const STARTUP_RETRY_INTERVAL: Duration = Duration::from_secs(30);
/// Attempts (the startup probe included) before the sensor gives up.
pub const STARTUP_MAX_ATTEMPTS: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
enum PmicState {
    /// The PMIC has answered at least once: every sample reads it.
    Active,
    /// The PMIC has never answered yet: attempts are spaced out and bounded.
    Pending {
        attempts: u32,
        next_attempt: Instant,
    },
    /// Every startup attempt failed: the firmware is no longer queried.
    GivenUp(String),
}

/// Measured board power of a Raspberry Pi 5, reported on the `cpu` sensor so
/// that the Hub, alerts and history keep working unchanged.
pub struct RpiPmicSensor<S: GencmdSource> {
    source: S,
    state: PmicState,
}

impl<S: GencmdSource> RpiPmicSensor<S> {
    /// A sensor whose PMIC has already answered a first read.
    pub fn new(source: S) -> Self {
        Self {
            source,
            state: PmicState::Active,
        }
    }

    /// A sensor for a Raspberry Pi 5 whose PMIC did not answer the startup
    /// probe. It retries every `STARTUP_RETRY_INTERVAL`, up to
    /// `STARTUP_MAX_ATTEMPTS` attempts in total, instead of being dropped
    /// until the next restart.
    pub fn pending(source: S, now: Instant) -> Self {
        Self {
            source,
            state: PmicState::Pending {
                attempts: 1,
                next_attempt: now + STARTUP_RETRY_INTERVAL,
            },
        }
    }

    fn read(&mut self) -> Result<SensorReading, SensorError> {
        let power = read_pmic_power(&mut self.source)?;

        Ok(SensorReading {
            component: Component::Board,
            watts: power.watts,
            confidence: Confidence::Measured,
            timestamp: chrono::Utc::now(),
        })
    }

    /// `sample` with an explicit clock, so that the retry schedule is testable.
    pub fn sample_at(&mut self, now: Instant) -> Result<SensorReading, SensorError> {
        match &self.state {
            PmicState::Active => self.read(),
            PmicState::GivenUp(message) => Err(SensorError::Unavailable(message.clone())),
            PmicState::Pending {
                attempts,
                next_attempt,
            } => {
                let attempts = *attempts;
                if now < *next_attempt {
                    return Err(SensorError::NotReady(format!(
                        "the PMIC has not answered yet (attempt {attempts} of \
                         {STARTUP_MAX_ATTEMPTS} failed, retrying)"
                    )));
                }

                match self.read() {
                    Ok(reading) => {
                        self.state = PmicState::Active;
                        Ok(reading)
                    }
                    Err(error) => {
                        let attempts = attempts + 1;
                        let cause = describe(&error);
                        if attempts >= STARTUP_MAX_ATTEMPTS {
                            let message = format!(
                                "the PMIC did not answer after {attempts} attempts, \
                                 giving up until the next restart (last error: {cause})"
                            );
                            self.state = PmicState::GivenUp(message.clone());
                            Err(SensorError::Unavailable(message))
                        } else {
                            self.state = PmicState::Pending {
                                attempts,
                                next_attempt: now + STARTUP_RETRY_INTERVAL,
                            };
                            Err(SensorError::NotReady(format!(
                                "the PMIC has not answered yet (attempt {attempts} of \
                                 {STARTUP_MAX_ATTEMPTS} failed: {cause})"
                            )))
                        }
                    }
                }
            }
        }
    }
}

fn describe(error: &SensorError) -> &str {
    match error {
        SensorError::PermissionDenied(message)
        | SensorError::Unavailable(message)
        | SensorError::ReadFailed(message)
        | SensorError::NotReady(message) => message,
    }
}

/// Runs `pmic_read_adc` once and returns the validated total.
pub fn read_pmic_power<S: GencmdSource>(source: &mut S) -> Result<PmicPower, SensorError> {
    let answer = source.run(PMIC_COMMAND)?;
    parse_pmic_adc(&answer)
}

impl<S: GencmdSource> PowerSensor for RpiPmicSensor<S> {
    fn sample(&mut self) -> Result<SensorReading, SensorError> {
        self.sample_at(Instant::now())
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
        let with_input = parse_pmic_rails(
            "VDD_CORE_A current(7)=1.0A\nVDD_CORE_V volt(15)=0.8V\nEXT5V_V volt(24)=5.1V\n",
        )
        .unwrap()
        .power();

        assert_eq!(with_input.rails, 1);
        assert!((with_input.watts - 0.8).abs() < 1e-12);
    }

    #[test]
    fn rails_can_appear_in_any_order() {
        let power = parse_pmic_rails(
            "X_V volt(1)=2.0V\nY_A current(2)=3.0A\nX_A current(0)=0.5A\nY_V volt(3)=1.0V\n",
        )
        .unwrap()
        .power();

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

    /// `PANDA_1` without the lines whose label is in `removed`.
    fn panda_without(removed: &[&str]) -> String {
        PANDA_1
            .lines()
            .filter(|line| {
                let label = line.split_whitespace().next().unwrap_or("");
                !removed.contains(&label)
            })
            .map(|line| format!("{line}\n"))
            .collect()
    }

    /// `PANDA_1` with the value of the line `label` replaced.
    fn panda_with(label: &str, value: &str) -> String {
        PANDA_1
            .lines()
            .map(|line| {
                if line.split_whitespace().next() == Some(label) {
                    let (head, _) = line.split_once('=').unwrap();
                    format!("{head}={value}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect()
    }

    fn assert_read_failed(answer: &str, expected: &str) {
        match parse_pmic_adc(answer) {
            Err(SensorError::ReadFailed(message)) => {
                assert!(message.contains(expected), "{message}")
            }
            other => panic!("expected a read failure about {expected:?}, got {other:?}"),
        }
    }

    #[test]
    fn the_test_helpers_leave_a_real_answer_valid() {
        assert_eq!(parse_pmic_adc(&panda_without(&[])).unwrap().rails, 12);
        assert_eq!(
            parse_pmic_adc(&panda_with("HDMI_A", "0.01709400A")).unwrap(),
            parse_pmic_adc(PANDA_1).unwrap()
        );
    }

    #[test]
    fn a_single_rail_is_not_a_board_reading() {
        assert_read_failed(
            "VDD_CORE_A current(7)=1.0A\nVDD_CORE_V volt(15)=0.8V\n",
            "is missing",
        );
    }

    #[test]
    fn a_missing_main_rail_is_rejected() {
        for rail in REQUIRED_RAILS {
            let current = format!("{rail}_A");
            let voltage = format!("{rail}_V");
            assert_read_failed(
                &panda_without(&[current.as_str(), voltage.as_str()]),
                &format!("rail {rail} is missing"),
            );
        }
    }

    #[test]
    fn a_current_without_its_voltage_is_rejected() {
        // The rail would otherwise be dropped silently from the sum.
        assert_read_failed(
            &panda_without(&["HDMI_V"]),
            "rail HDMI has a current but no voltage",
        );
    }

    #[test]
    fn an_answer_cut_after_the_currents_is_rejected() {
        let currents_only: String = PANDA_1
            .lines()
            .filter(|line| line.contains("current("))
            .map(|line| format!("{line}\n"))
            .collect();

        assert_read_failed(&currents_only, "has a current but no voltage");
    }

    #[test]
    fn too_few_complete_rails_are_rejected() {
        // The four main rails plus three others: 7 complete rails.
        let seven = panda_without(&[
            "3V7_WL_SW_A",
            "3V7_WL_SW_V",
            "DDR_VDDQ_A",
            "DDR_VDDQ_V",
            "3V3_DAC_A",
            "3V3_DAC_V",
            "3V3_ADC_A",
            "3V3_ADC_V",
            "0V8_AON_A",
            "0V8_AON_V",
        ]);
        assert_eq!(parse_pmic_rails(&seven).unwrap().power().rails, 7);
        assert_read_failed(&seven, "7 complete rails");

        // One more rail reaches the minimum.
        let eight = panda_without(&[
            "3V7_WL_SW_A",
            "3V7_WL_SW_V",
            "DDR_VDDQ_A",
            "DDR_VDDQ_V",
            "3V3_DAC_A",
            "3V3_DAC_V",
            "3V3_ADC_A",
            "3V3_ADC_V",
        ]);
        assert_eq!(parse_pmic_adc(&eight).unwrap().rails, MIN_COMPLETE_RAILS);
    }

    #[test]
    fn implausible_rail_values_are_rejected() {
        assert_read_failed(
            &panda_with("VDD_CORE_A", "-0.50000000A"),
            "implausible PMIC current for VDD_CORE",
        );
        assert_read_failed(
            &panda_with("VDD_CORE_A", "25.00000000A"),
            "implausible PMIC current for VDD_CORE",
        );
        assert_read_failed(
            &panda_with("3V3_SYS_V", "33.10033000V"),
            "implausible PMIC voltage for 3V3_SYS",
        );
        assert_read_failed(
            &panda_with("EXT5V_V", "-5.12416000V"),
            "implausible PMIC voltage for EXT5V",
        );
    }

    #[test]
    fn an_implausible_total_is_rejected() {
        let all_zero: String = PANDA_1
            .lines()
            .map(|line| match line.split_once('=') {
                Some((head, _)) if line.contains("current(") => format!("{head}=0.00000000A\n"),
                _ => format!("{line}\n"),
            })
            .collect();
        assert_read_failed(&all_zero, "implausible PMIC total");

        let too_high = panda_with("3V3_SYS_A", "9.90000000A");
        assert_read_failed(&too_high, "implausible PMIC total");
    }

    #[test]
    fn a_rail_reported_twice_is_rejected() {
        let twice = format!("{PANDA_1}  VDD_CORE_A current(7)=0.10000000A\n");

        assert_read_failed(&twice, "VDD_CORE_A is reported twice");
    }

    #[test]
    fn a_rail_value_without_its_unit_is_rejected() {
        assert_read_failed(
            &panda_with("VDD_CORE_A", "0.79477000"),
            "does not end with A",
        );
    }

    #[test]
    fn non_finite_values_are_rejected() {
        assert_read_failed(&panda_with("VDD_CORE_A", "NaNA"), "non-finite");
        assert_read_failed(&panda_with("VDD_CORE_V", "infV"), "non-finite");
    }

    #[test]
    fn pi5_family_boards_are_recognised_by_their_model() {
        for model in [
            "Raspberry Pi 5 Model B Rev 1.0",
            "Raspberry Pi 5 Model B Rev 1.1\0",
            "Raspberry Pi Compute Module 5 Rev 1.0",
            "Raspberry Pi 500 Rev 1.0",
        ] {
            assert!(is_pi5_family(model), "{model}");
        }
        for model in [
            "Raspberry Pi 4 Model B Rev 1.1",
            "Raspberry Pi 3 Model A Plus Rev 1.0",
            "Raspberry Pi 2 Model B Rev 1.1",
            "Raspberry Pi Compute Module 4 Rev 1.1",
            "Hardkernel ODROID-N2Plus",
            "",
        ] {
            assert!(!is_pi5_family(model), "{model}");
        }
    }

    #[test]
    fn the_board_model_is_read_from_cpuinfo() {
        let cpuinfo = "processor\t: 3\nBogoMIPS\t: 108.00\n\nRevision\t: d04170\n\
                       Serial\t\t: 0000\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n";

        assert_eq!(
            model_from_cpuinfo(cpuinfo),
            Some("Raspberry Pi 5 Model B Rev 1.0")
        );
        assert_eq!(
            model_from_cpuinfo("model name\t: Intel(R) Core(TM)\n"),
            None
        );
    }

    /// Answers in order, then keeps repeating the last one.
    struct ScriptedGencmd {
        answers: Vec<Result<String, SensorError>>,
        calls: usize,
    }

    impl ScriptedGencmd {
        fn new(answers: Vec<Result<String, SensorError>>) -> Self {
            Self { answers, calls: 0 }
        }
    }

    impl GencmdSource for ScriptedGencmd {
        fn run(&mut self, _command: &str) -> Result<String, SensorError> {
            let index = self.calls.min(self.answers.len() - 1);
            self.calls += 1;
            self.answers[index].clone()
        }
    }

    fn busy() -> Result<String, SensorError> {
        Err(SensorError::ReadFailed("mailbox busy".to_string()))
    }

    #[test]
    fn a_pending_sensor_waits_for_the_retry_interval() {
        let start = Instant::now();
        let mut sensor =
            RpiPmicSensor::pending(ScriptedGencmd::new(vec![Ok(PANDA_1.to_string())]), start);

        let early = sensor.sample_at(start + STARTUP_RETRY_INTERVAL - Duration::from_secs(1));

        assert!(matches!(early, Err(SensorError::NotReady(_))), "{early:?}");
        assert_eq!(sensor.source.calls, 0, "the firmware is not queried early");
    }

    #[test]
    fn a_pending_sensor_recovers_when_the_pmic_answers() {
        let start = Instant::now();
        let mut sensor = RpiPmicSensor::pending(
            ScriptedGencmd::new(vec![busy(), Ok(PANDA_1.to_string())]),
            start,
        );

        let second = sensor.sample_at(start + STARTUP_RETRY_INTERVAL);
        assert!(
            matches!(&second, Err(SensorError::NotReady(m)) if m.contains("attempt 2 of 10") && m.contains("mailbox busy")),
            "{second:?}"
        );

        let third = sensor
            .sample_at(start + STARTUP_RETRY_INTERVAL * 2)
            .unwrap();
        assert_eq!(third.confidence, Confidence::Measured);
        assert!((third.watts - 2.182_264_594).abs() < 1e-6);

        // Once active, every sample reads the PMIC, with no waiting.
        sensor
            .sample_at(start + STARTUP_RETRY_INTERVAL * 2)
            .unwrap();
        assert_eq!(sensor.source.calls, 3);
    }

    #[test]
    fn a_pending_sensor_gives_up_after_the_last_attempt() {
        let start = Instant::now();
        let mut sensor = RpiPmicSensor::pending(ScriptedGencmd::new(vec![busy()]), start);

        // The startup probe was attempt 1; attempts 2 to 9 keep waiting.
        for attempt in 2..STARTUP_MAX_ATTEMPTS {
            let result = sensor.sample_at(start + STARTUP_RETRY_INTERVAL * (attempt - 1));
            assert!(
                matches!(result, Err(SensorError::NotReady(_))),
                "attempt {attempt}: {result:?}"
            );
        }

        let last = sensor.sample_at(start + STARTUP_RETRY_INTERVAL * (STARTUP_MAX_ATTEMPTS - 1));
        assert!(
            matches!(&last, Err(SensorError::Unavailable(m)) if m.contains("after 10 attempts") && m.contains("mailbox busy")),
            "{last:?}"
        );
        let calls = sensor.source.calls;
        assert_eq!(calls as u32, STARTUP_MAX_ATTEMPTS - 1);

        // Afterwards the firmware is left alone.
        let later = sensor.sample_at(start + STARTUP_RETRY_INTERVAL * 100);
        assert!(matches!(later, Err(SensorError::Unavailable(_))));
        assert_eq!(sensor.source.calls, calls);
    }

    #[test]
    fn an_active_sensor_keeps_reading_after_a_failed_sample() {
        let mut sensor = RpiPmicSensor::new(ScriptedGencmd::new(vec![
            Ok(PANDA_1.to_string()),
            busy(),
            Ok(PANDA_2.to_string()),
        ]));
        let now = Instant::now();

        assert!(sensor.sample_at(now).is_ok());
        assert!(matches!(
            sensor.sample_at(now),
            Err(SensorError::ReadFailed(_))
        ));
        let recovered = sensor.sample_at(now).unwrap();
        assert!((recovered.watts - 2.119_045_919).abs() < 1e-6);
    }

    #[test]
    fn an_incomplete_answer_fails_the_sample() {
        let mut sensor = RpiPmicSensor::new(ScriptedGencmd::new(vec![Ok(panda_without(&[
            "VDD_CORE_A",
            "VDD_CORE_V",
        ]))]));

        assert!(matches!(
            sensor.sample().unwrap_err(),
            SensorError::ReadFailed(_)
        ));
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
        assert_eq!(reading.component, Component::Board);
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
