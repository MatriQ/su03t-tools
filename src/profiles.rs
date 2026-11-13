//! Presets describe a particular application or wiring mode, never the flash protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Car,
    B8,
    Custom,
}
impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Car => "小车固件 · 20261107001",
            Self::B8 => "B8 启动日志 · 只接收",
            Self::Custom => "通用串口 · 自定义",
        }
    }
    pub fn baud(self) -> u32 {
        match self {
            Self::Car => 9600,
            Self::B8 => 57600,
            Self::Custom => 115200,
        }
    }
    pub fn receive_only(self) -> bool {
        self == Self::B8
    }
    pub fn wiring(self) -> &'static str {
        match self {
            Self::Car => "B7 → RXD · B6 ← TXD · GND 共地",
            Self::B8 => "B8 → RXD · GND 共地 · 转接器 TXD 不接",
            Self::Custom => "模块 TX → RXD · 模块 RX ← TXD · GND 共地；引脚按固件配置",
        }
    }
}

pub const CAR_COMMANDS: [(u8, &str); 5] = [
    (0x01, "前进"),
    (0x02, "后退"),
    (0x03, "左转"),
    (0x04, "右转"),
    (0x00, "停止"),
];
#[derive(Default)]
pub struct CarTracker {
    pub counts: [u64; 5],
    pub unknown: u64,
    pub last: Option<&'static str>,
}
impl CarTracker {
    pub fn observe(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if let Some((index, (_, name))) = CAR_COMMANDS
                .iter()
                .enumerate()
                .find(|(_, (code, _))| code == byte)
            {
                self.counts[index] = self.counts[index].saturating_add(1);
                self.last = Some(name);
            } else {
                self.unknown = self.unknown.saturating_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_car_bytes_are_counted_without_framing_or_ignoring_zero() {
        let mut tracker = CarTracker::default();
        tracker.observe(&[1, 2, 3, 4, 0, 0, 0x80, 0xff]);
        assert_eq!(tracker.counts, [1, 1, 1, 1, 2]);
        assert_eq!(tracker.last, Some("停止"));
        assert_eq!(tracker.unknown, 2);
        assert_eq!(Profile::B8.baud(), 57600);
        assert!(Profile::B8.receive_only());
        assert_eq!(Profile::Car.baud(), 9600);
    }
}
