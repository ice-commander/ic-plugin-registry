#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    String(String),
    ExpandString(String),
    MultiString(Vec<String>),
    DWord(u32),
    QWord(u64),
    Binary(Vec<u8>),
    Unknown,
}

impl Data {
    pub fn type_name(&self) -> &'static str {
        match self {
            Data::String(_) => "REG_SZ",
            Data::ExpandString(_) => "REG_EXPAND_SZ",
            Data::MultiString(_) => "REG_MULTI_SZ",
            Data::DWord(_) => "REG_DWORD",
            Data::QWord(_) => "REG_QWORD",
            Data::Binary(_) => "REG_BINARY",
            Data::Unknown => "REG_NONE",
        }
    }

    pub fn from_type_name(name: &str, text: &str) -> Result<Data, String> {
        match name {
            "REG_SZ" => Ok(Data::String(text.to_string())),
            "REG_EXPAND_SZ" => Ok(Data::ExpandString(text.to_string())),
            "REG_MULTI_SZ" => Ok(Data::MultiString(
                text.lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string())
                    .collect(),
            )),
            "REG_DWORD" => parse_number(text)
                .and_then(|n| u32::try_from(n).map_err(|_| "value does not fit in 32 bits".into()))
                .map(Data::DWord),
            "REG_QWORD" => parse_number(text).map(Data::QWord),
            "REG_BINARY" => parse_hex(text).map(Data::Binary),
            other => Err(format!("unknown type {other}")),
        }
    }

    pub fn shown(&self) -> String {
        match self {
            Data::String(s) | Data::ExpandString(s) => s.clone(),
            Data::MultiString(parts) => parts.join(" "),
            Data::DWord(n) => format!("0x{n:08x} ({n})"),
            Data::QWord(n) => format!("0x{n:016x} ({n})"),
            Data::Binary(bytes) => bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<String>>()
                .join(" "),
            Data::Unknown => String::new(),
        }
    }

    pub fn editable(&self) -> String {
        match self {
            Data::MultiString(parts) => parts.join("\n"),
            Data::DWord(n) => n.to_string(),
            Data::QWord(n) => n.to_string(),
            _ => self.shown(),
        }
    }
}

fn parse_number(text: &str) -> Result<u64, String> {
    let trimmed = text.trim();
    let parsed = match trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => trimmed.parse::<u64>(),
    };
    parsed.map_err(|_| format!("{trimmed} is not a number"))
}

fn parse_hex(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() % 2 != 0 {
        return Err("hex needs an even number of digits".to_string());
    }
    digits
        .chunks(2)
        .map(|pair| match (pair[0].to_digit(16), pair[1].to_digit(16)) {
            (Some(high), Some(low)) => Ok((high << 4 | low) as u8),
            _ => Err(format!("{}{} is not hex", pair[0], pair[1])),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Value {
    pub name: String,
    pub data: Data,
}

#[derive(Debug, Clone, Default)]
pub struct Listing {
    pub keys: Vec<String>,
    pub values: Vec<Value>,
    pub error: Option<String>,
}

#[cfg(target_os = "windows")]
pub const ROOTS: [&str; 5] = ["HKLM", "HKCU", "HKCR", "HKU", "HKCC"];

#[cfg(target_os = "windows")]
mod windows {
    use super::{Data, Listing, Value, ROOTS};
    use winreg::enums::*;
    use winreg::RegKey;

    fn split(path: &str) -> Option<(RegKey, String)> {
        let parts: Vec<&str> = path.trim_matches('/').splitn(2, '/').collect();
        let root = match parts.first()?.to_uppercase().as_str() {
            "HKLM" => HKEY_LOCAL_MACHINE,
            "HKCU" => HKEY_CURRENT_USER,
            "HKCR" => HKEY_CLASSES_ROOT,
            "HKU" => HKEY_USERS,
            "HKCC" => HKEY_CURRENT_CONFIG,
            _ => return None,
        };
        let rest = parts.get(1).map(|tail| tail.replace('/', "\\")).unwrap_or_default();
        Some((RegKey::predef(root), rest))
    }

    fn wide(bytes: &[u8]) -> String {
        let pairs: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&pairs)
    }

    fn data_of(value: &winreg::RegValue) -> Data {
        match value.vtype {
            REG_SZ => Data::String(wide(&value.bytes).trim_end_matches('\0').to_string()),
            REG_EXPAND_SZ => {
                Data::ExpandString(wide(&value.bytes).trim_end_matches('\0').to_string())
            }
            REG_MULTI_SZ => Data::MultiString(
                wide(&value.bytes)
                    .split('\0')
                    .filter(|part| !part.is_empty())
                    .map(|part| part.to_string())
                    .collect(),
            ),
            REG_DWORD if value.bytes.len() >= 4 => {
                let mut held = [0u8; 4];
                held.copy_from_slice(&value.bytes[..4]);
                Data::DWord(u32::from_le_bytes(held))
            }
            REG_QWORD if value.bytes.len() >= 8 => {
                let mut held = [0u8; 8];
                held.copy_from_slice(&value.bytes[..8]);
                Data::QWord(u64::from_le_bytes(held))
            }
            REG_BINARY => Data::Binary(value.bytes.clone()),
            _ => Data::Unknown,
        }
    }

    pub fn read(path: &str) -> Listing {
        if path.is_empty() || path == "/" {
            return Listing {
                keys: ROOTS.iter().map(|root| root.to_string()).collect(),
                ..Listing::default()
            };
        }
        let Some((root, rest)) = split(path) else {
            return Listing {
                error: Some("unknown root key".to_string()),
                ..Listing::default()
            };
        };
        match root.open_subkey_with_flags(&rest, KEY_READ) {
            Ok(key) => {
                let mut keys: Vec<String> = key.enum_keys().filter_map(|name| name.ok()).collect();
                keys.sort_by_key(|name| name.to_lowercase());
                let mut values: Vec<Value> = key
                    .enum_values()
                    .filter_map(|found| found.ok())
                    .map(|(name, value)| Value {
                        name,
                        data: data_of(&value),
                    })
                    .collect();
                values.sort_by_key(|value| value.name.to_lowercase());
                Listing {
                    keys,
                    values,
                    error: None,
                }
            }
            Err(e) => Listing {
                error: Some(e.to_string()),
                ..Listing::default()
            },
        }
    }

    fn encode(data: &Data) -> Option<winreg::RegValue> {
        let (vtype, bytes) = match data {
            Data::String(s) => (REG_SZ, wide_bytes(s, true)),
            Data::ExpandString(s) => (REG_EXPAND_SZ, wide_bytes(s, true)),
            Data::MultiString(parts) => {
                let mut bytes = Vec::new();
                for part in parts {
                    bytes.extend(wide_bytes(part, true));
                }
                bytes.extend_from_slice(&[0, 0]);
                (REG_MULTI_SZ, bytes)
            }
            Data::DWord(n) => (REG_DWORD, n.to_le_bytes().to_vec()),
            Data::QWord(n) => (REG_QWORD, n.to_le_bytes().to_vec()),
            Data::Binary(bytes) => (REG_BINARY, bytes.clone()),
            Data::Unknown => return None,
        };
        Some(winreg::RegValue { bytes, vtype })
    }

    fn wide_bytes(text: &str, terminate: bool) -> Vec<u8> {
        let mut pairs: Vec<u16> = text.encode_utf16().collect();
        if terminate {
            pairs.push(0);
        }
        pairs.iter().flat_map(|unit| unit.to_le_bytes()).collect()
    }

    pub fn set_value(path: &str, name: &str, data: &Data) -> Result<(), String> {
        let Some((root, rest)) = split(path) else {
            return Err("unknown root key".to_string());
        };
        let Some(encoded) = encode(data) else {
            return Err("nothing to write for this type".to_string());
        };
        if rest.is_empty() {
            return root.set_raw_value(name, &encoded).map_err(|e| e.to_string());
        }
        root.open_subkey_with_flags(&rest, KEY_WRITE | KEY_READ)
            .and_then(|key| key.set_raw_value(name, &encoded))
            .map_err(|e| e.to_string())
    }

    pub fn create_key(path: &str, name: &str) -> Result<(), String> {
        let Some((root, rest)) = split(path) else {
            return Err("unknown root key".to_string());
        };
        let under = if rest.is_empty() {
            name.to_string()
        } else {
            format!("{rest}\\{name}")
        };
        root.create_subkey(&under).map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn delete(path: &str, is_key: bool, value_name: Option<&str>) -> Result<(), String> {
        let Some((root, rest)) = split(path) else {
            return Err("unknown root key".to_string());
        };
        if rest.is_empty() {
            if is_key {
                return Err("cannot delete a root key".to_string());
            }
            let Some(name) = value_name else {
                return Err("no value named to delete".to_string());
            };
            return root.delete_value(name).map_err(|e| e.to_string());
        }
        if is_key {
            return match rest.rfind('\\') {
                Some(at) => root
                    .open_subkey_with_flags(&rest[..at], KEY_WRITE | KEY_READ)
                    .and_then(|parent| parent.delete_subkey(&rest[at + 1..]))
                    .map_err(|e| e.to_string()),
                None => root.delete_subkey(&rest).map_err(|e| e.to_string()),
            };
        }
        let Some(name) = value_name else {
            return Err("no value named to delete".to_string());
        };
        root.open_subkey_with_flags(&rest, KEY_WRITE | KEY_READ)
            .and_then(|key| key.delete_value(name))
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(target_os = "windows"))]
mod windows {
    use super::{Data, Listing};

    const ELSEWHERE: &str = "the registry is a Windows thing";

    pub fn read(_path: &str) -> Listing {
        Listing {
            error: Some(ELSEWHERE.to_string()),
            ..Listing::default()
        }
    }
    pub fn set_value(_path: &str, _name: &str, _data: &Data) -> Result<(), String> {
        Err(ELSEWHERE.to_string())
    }
    pub fn create_key(_path: &str, _name: &str) -> Result<(), String> {
        Err(ELSEWHERE.to_string())
    }
    pub fn delete(_path: &str, _is_key: bool, _value_name: Option<&str>) -> Result<(), String> {
        Err(ELSEWHERE.to_string())
    }
}

pub use windows::{create_key, delete, read, set_value};

pub fn child_of(path: &str, name: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        name.to_string()
    } else {
        format!("{trimmed}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes to the real HKCU, the one place a test may write.
    #[cfg(target_os = "windows")]
    #[test]
    fn a_value_goes_into_a_hive_root_and_comes_out_again() {
        const NAME: &str = "ic-registry-dlg selftest";
        let written = Data::String("selftest".to_string());
        set_value("HKCU", NAME, &written).expect("a hive root takes values");

        let found = read("HKCU").values.into_iter().find(|had| had.name == NAME);
        assert_eq!(found.map(|had| had.data), Some(written));

        delete("HKCU", false, Some(NAME)).expect("and gives them up again");
        assert!(read("HKCU").values.iter().all(|had| had.name != NAME));
    }

    #[test]
    fn a_type_name_round_trips_through_its_text() {
        for (data, text) in [
            (Data::String("hello".into()), "hello"),
            (Data::ExpandString("%PATH%".into()), "%PATH%"),
            (Data::DWord(4), "4"),
            (Data::QWord(u64::MAX), "18446744073709551615"),
        ] {
            let back = Data::from_type_name(data.type_name(), text).expect("parses");
            assert_eq!(back, data);
        }
    }

    #[test]
    fn a_multi_string_is_one_line_per_entry() {
        let data = Data::MultiString(vec!["one".into(), "two".into()]);
        assert_eq!(data.editable(), "one\ntwo");
        assert_eq!(
            Data::from_type_name("REG_MULTI_SZ", "one\ntwo").expect("parses"),
            data
        );
    }

    #[test]
    fn empty_lines_do_not_become_empty_entries() {
        assert_eq!(
            Data::from_type_name("REG_MULTI_SZ", "\none\n\ntwo\n").expect("parses"),
            Data::MultiString(vec!["one".into(), "two".into()])
        );
    }

    #[test]
    fn non_ascii_in_hex_is_refused_rather_than_sliced() {
        assert!(Data::from_type_name("REG_BINARY", "a\u{e9}1").is_err());
        assert!(Data::from_type_name("REG_BINARY", "a\u{e9}").is_err());
        assert!(Data::from_type_name("REG_BINARY", "+f").is_err());
    }

    #[test]
    fn binary_reads_and_writes_as_spaced_hex() {
        let data = Data::Binary(vec![0x00, 0x1f, 0xff]);
        assert_eq!(data.shown(), "00 1f ff");
        assert_eq!(
            Data::from_type_name("REG_BINARY", "00 1f ff").expect("parses"),
            data
        );
        assert!(Data::from_type_name("REG_BINARY", "0").is_err());
        assert!(Data::from_type_name("REG_BINARY", "zz").is_err());
    }

    #[test]
    fn a_number_is_accepted_in_decimal_or_hex() {
        assert_eq!(
            Data::from_type_name("REG_DWORD", "0x10").expect("parses"),
            Data::DWord(16)
        );
        assert_eq!(
            Data::from_type_name("REG_DWORD", "16").expect("parses"),
            Data::DWord(16)
        );
        assert!(Data::from_type_name("REG_DWORD", "4294967296").is_err());
        assert!(Data::from_type_name("REG_DWORD", "nonsense").is_err());
    }

    #[test]
    fn walking_down_from_the_roots_does_not_double_the_separator() {
        assert_eq!(child_of("", "HKLM"), "HKLM");
        assert_eq!(child_of("/", "HKLM"), "HKLM");
        assert_eq!(child_of("HKLM", "SOFTWARE"), "HKLM/SOFTWARE");
    }
}
