//! The `SET_CONFIG` payload codec — a Rust mirror of the HDL `Tamal.Config`.

use crate::isa::Cfg6;

/// Link role. v1 is controller-only; `Target` is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Drive the bus as the eSPI controller (v1).
    Controller,
    /// Act as the eSPI target (reserved).
    Target,
}

/// I/O width. v1 is single-lane (`X1`); `X2`/`X4` land in Phase 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoMode {
    /// Single I/O (v1).
    X1,
    /// Dual I/O (reserved).
    X2,
    /// Quad I/O (reserved).
    X4,
}

/// SCK frequency selection. v1 accepts only 20 MHz (`Sck20`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sck {
    /// 20 MHz (v1).
    Sck20,
    /// 33 MHz (reserved).
    Sck33,
    /// 50 MHz (reserved).
    Sck50,
    /// 66 MHz (reserved).
    Sck66,
}

/// Where alerts are observed: the dedicated `ALERT#` pin or in-band on `IO[1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertSource {
    /// The dedicated `ALERT#` pin.
    AlertPin,
    /// In-band alerts on `IO[1]`.
    AlertIo1,
}

/// The decoded engine configuration (one field per `SET_CONFIG` sub-field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Link role.
    pub role: Role,
    /// I/O width.
    pub io_mode: IoMode,
    /// SCK frequency.
    pub sck: Sck,
    /// Alert observation source.
    pub alert_source: AlertSource,
}

/// Why a `SET_CONFIG` payload was rejected (each becomes a TRAP in the engine).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The selected role is not supported in v1.
    #[error("unsupported role (v1 is controller-only)")]
    UnsupportedRole,
    /// The selected I/O mode is not supported in v1.
    #[error("unsupported I/O mode (v1 is single-lane)")]
    UnsupportedIoMode,
    /// The selected SCK frequency is not supported in v1.
    #[error("unsupported SCK frequency (v1 is 20 MHz)")]
    UnsupportedSck,
}

impl Config {
    /// Pack into the 6-bit `SET_CONFIG` payload: `[5]=role · [4:3]=io_mode ·
    /// [2:1]=sck · [0]=alert_source`. Total (any `Config` packs).
    pub fn pack(&self) -> Cfg6 {
        let role = match self.role {
            Role::Controller => 0u8,
            Role::Target => 1,
        };
        let io = match self.io_mode {
            IoMode::X1 => 0u8,
            IoMode::X2 => 1,
            IoMode::X4 => 2,
        };
        let sck = match self.sck {
            Sck::Sck20 => 0u8,
            Sck::Sck33 => 1,
            Sck::Sck50 => 2,
            Sck::Sck66 => 3,
        };
        let alert = match self.alert_source {
            AlertSource::AlertPin => 0u8,
            AlertSource::AlertIo1 => 1,
        };
        Cfg6::from_bits((role << 5) | (io << 3) | (sck << 1) | alert)
    }
}

/// Decode a 6-bit `SET_CONFIG` payload into a [`Config`], v1-strict: only
/// `(Controller, X1, Sck20, *)` is accepted, matching the HDL `decodeConfig`.
pub fn decode_config(payload: Cfg6) -> Result<Config, ConfigError> {
    let p = payload.bits();
    let role = (p >> 5) & 0x1;
    let io = (p >> 3) & 0x3;
    let sck = (p >> 1) & 0x3;
    let alert = p & 0x1;
    match (role, io, sck) {
        (0b0, 0b00, 0b00) => Ok(Config {
            role: Role::Controller,
            io_mode: IoMode::X1,
            sck: Sck::Sck20,
            alert_source: if alert == 0 {
                AlertSource::AlertPin
            } else {
                AlertSource::AlertIo1
            },
        }),
        (0b1, _, _) => Err(ConfigError::UnsupportedRole),
        (_, io_, _) if io_ != 0b00 => Err(ConfigError::UnsupportedIoMode),
        _ => Err(ConfigError::UnsupportedSck),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isa::Cfg6;

    #[test]
    fn pack_places_fields_at_pinned_bits() {
        let base = Config {
            role: Role::Controller,
            io_mode: IoMode::X1,
            sck: Sck::Sck20,
            alert_source: AlertSource::AlertPin,
        };
        assert_eq!(base.pack().bits(), 0x00);
        assert_eq!(
            Config {
                alert_source: AlertSource::AlertIo1,
                ..base
            }
            .pack()
            .bits(),
            0x01
        );
        assert_eq!(
            Config {
                sck: Sck::Sck33,
                ..base
            }
            .pack()
            .bits(),
            0x02
        );
        assert_eq!(
            Config {
                io_mode: IoMode::X2,
                ..base
            }
            .pack()
            .bits(),
            0x08
        );
        assert_eq!(
            Config {
                role: Role::Target,
                ..base
            }
            .pack()
            .bits(),
            0x20
        );
    }

    #[test]
    fn decode_config_accepts_only_v1() {
        let c = decode_config(Cfg6::new(0x00).unwrap()).unwrap();
        assert!(matches!(c.role, Role::Controller));
        assert!(matches!(c.io_mode, IoMode::X1));
        assert!(matches!(c.sck, Sck::Sck20));
        assert!(matches!(c.alert_source, AlertSource::AlertPin));
        let c1 = decode_config(Cfg6::new(0x01).unwrap()).unwrap();
        assert!(matches!(c1.alert_source, AlertSource::AlertIo1));
    }

    #[test]
    fn decode_config_rejects_non_v1_in_priority_order() {
        // role bit set -> UnsupportedRole (regardless of io/sck)
        assert_eq!(
            decode_config(Cfg6::new(0x20).unwrap()),
            Err(ConfigError::UnsupportedRole)
        );
        // role ok, io != 0 -> UnsupportedIoMode
        assert_eq!(
            decode_config(Cfg6::new(0x08).unwrap()),
            Err(ConfigError::UnsupportedIoMode)
        );
        // role/io ok, sck != 0 -> UnsupportedSck
        assert_eq!(
            decode_config(Cfg6::new(0x02).unwrap()),
            Err(ConfigError::UnsupportedSck)
        );
    }

    #[test]
    fn decode_config_round_trips_v1() {
        for alert in [AlertSource::AlertPin, AlertSource::AlertIo1] {
            let c = Config {
                role: Role::Controller,
                io_mode: IoMode::X1,
                sck: Sck::Sck20,
                alert_source: alert,
            };
            assert_eq!(decode_config(c.pack()), Ok(c));
        }
    }
}
