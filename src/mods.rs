use std::fmt;
use std::ops::BitOr;
use std::str::FromStr;

/// A modifier bitmask: X's layout, which duckwm uses on both backends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods(pub u32);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1 << 0);
    pub const LOCK: Mods = Mods(1 << 1);
    pub const CTRL: Mods = Mods(1 << 2);
    pub const ALT: Mods = Mods(1 << 3);
    pub const MOD2: Mods = Mods(1 << 4);
    pub const MOD3: Mods = Mods(1 << 5);
    pub const SUPER: Mods = Mods(1 << 6);
    pub const MOD5: Mods = Mods(1 << 7);

    /// Every modifier, in the order combos are written: "super+ctrl+alt+shift+q".
    pub const ALL: [(&'static str, Mods); 8] = [
        ("super", Mods::SUPER),
        ("ctrl", Mods::CTRL),
        ("alt", Mods::ALT),
        ("shift", Mods::SHIFT),
        ("lock", Mods::LOCK),
        ("mod2", Mods::MOD2),
        ("mod3", Mods::MOD3),
        ("mod5", Mods::MOD5),
    ];

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    /// The names of the modifiers held, in [`Mods::ALL`]'s order.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Mods::ALL
            .into_iter()
            .filter(move |(_, m)| self.contains(*m))
            .map(|(name, _)| name)
    }

    /// One modifier by name, any case; "control" is ctrl.
    pub fn from_name(name: &str) -> Option<Mods> {
        let name = name.to_ascii_lowercase();
        let name = if name == "control" {
            "ctrl"
        } else {
            name.as_str()
        };
        Mods::ALL
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, m)| m)
    }
}

impl BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

/// A bound key: modifiers plus a keysym name ("q", "Return").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub mods: Mods,
    pub key: String,
}

impl fmt::Display for KeyCombo {
    /// "super+shift+q".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for name in self.mods.names() {
            write!(f, "{name}+")?;
        }
        f.write_str(&self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseComboError(pub String);

impl fmt::Display for ParseComboError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseComboError {}

impl FromStr for KeyCombo {
    type Err = ParseComboError;

    /// "super+shift+q": the key is the last part, the rest are modifiers.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (mods, key) = s.rsplit_once('+').unwrap_or(("", s));
        if key.is_empty() {
            return Err(ParseComboError(format!("missing key in {s:?}")));
        }
        let mut bits = Mods::NONE;
        for m in mods.split('+').filter(|m| !m.is_empty()) {
            bits = bits
                | Mods::from_name(m)
                    .ok_or_else(|| ParseComboError(format!("unknown modifier {m:?} in {s:?}")))?;
        }
        Ok(KeyCombo {
            mods: bits,
            key: key.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c: KeyCombo = "shift+Super+q".parse().unwrap();
        assert_eq!(c.mods, Mods::SUPER | Mods::SHIFT);
        assert_eq!(c.mods.0, 65);
        assert_eq!(c.to_string(), "super+shift+q");
        assert_eq!("Return".parse::<KeyCombo>().unwrap().to_string(), "Return");
    }

    #[test]
    fn rejects_junk() {
        assert!("super+".parse::<KeyCombo>().is_err());
        assert!("hyper+q".parse::<KeyCombo>().is_err());
    }
}
