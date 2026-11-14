//! The keys the agent presses, as Playwright's US keyboard layout describes them to Chromium (Input.dispatchKeyEvent):
//! key, code, the Windows key code and the text a key types.

/// key, code, Windows key code, text typed (none for a key that types nothing)
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub key: String,
    pub code: String,
    pub key_code: i64,
    pub text: Option<String>,
}

const ALT: i64 = 1;
const CONTROL: i64 = 2;
const META: i64 = 4;
const SHIFT: i64 = 8;

/// The modifier bit of a modifier key, 0 for any other.
pub fn modifier(key: &str) -> i64 {
    match key {
        "Alt" => ALT,
        "Control" => CONTROL,
        "Meta" => META,
        "Shift" => SHIFT,
        _ => 0,
    }
}

/// A key by its Playwright name ("Escape", "KeyS", "A", "@"); None for one this table does not know.
pub fn key(name: &str) -> Option<Key> {
    let k = |key: &str, code: &str, key_code: i64, text: Option<&str>| Key { key: key.into(), code: code.into(), key_code, text: text.map(String::from) };
    Some(match name {
        "Escape" => k("Escape", "Escape", 27, None),
        "Enter" => k("Enter", "Enter", 13, Some("\r")),
        "Delete" => k("Delete", "Delete", 46, None),
        "Backspace" => k("Backspace", "Backspace", 8, None),
        "Tab" => k("Tab", "Tab", 9, None),
        "ArrowUp" => k("ArrowUp", "ArrowUp", 38, None),
        "ArrowDown" => k("ArrowDown", "ArrowDown", 40, None),
        "Shift" => k("Shift", "ShiftLeft", 16, None),
        "Control" => k("Control", "ControlLeft", 17, None),
        "Alt" => k("Alt", "AltLeft", 18, None),
        "Meta" => k("Meta", "MetaLeft", 91, None),
        " " | "Space" => k(" ", "Space", 32, Some(" ")),
        "@" => k("@", "Digit2", 50, Some("@")),
        n if n.len() == 4 && n.starts_with("Key") && n.as_bytes()[3].is_ascii_uppercase() => {
            let c = n.as_bytes()[3] as char;
            let lower = c.to_ascii_lowercase().to_string();
            k(&lower, n, c as i64, Some(&lower))
        }
        n if n.len() == 1 && n.as_bytes()[0].is_ascii_alphabetic() => {
            let c = n.as_bytes()[0] as char;
            k(n, &format!("Key{}", c.to_ascii_uppercase()), c.to_ascii_uppercase() as i64, Some(n))
        }
        n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => k(n, &format!("Digit{n}"), n.as_bytes()[0] as i64, Some(n)),
        _ => return None,
    })
}

/// "Control+Shift+H": the modifiers held, then the key.
pub fn combo(keys: &str) -> Result<(Vec<Key>, Key), String> {
    let parts: Vec<&str> = if keys == "+" { vec!["+"] } else { keys.split('+').collect() };
    let (last, mods) = parts.split_last().ok_or("no key")?;
    let held = mods.iter().map(|m| key(m).filter(|k| modifier(&k.key) != 0).ok_or(format!("unknown modifier {m}"))).collect::<Result<Vec<_>, _>>()?;
    Ok((held, key(last).ok_or(format!("unknown key {last}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_combinations() {
        let (held, k) = combo("Alt+Shift+KeyS").expect("combo");
        assert_eq!(held.iter().map(|k| k.key.as_str()).collect::<Vec<_>>(), ["Alt", "Shift"]);
        assert_eq!((k.key.as_str(), k.code.as_str(), k.key_code), ("s", "KeyS", 83));
        let (_, a) = combo("Control+A").expect("combo");
        assert_eq!((a.key.as_str(), a.code.as_str(), a.key_code), ("A", "KeyA", 65));
        assert_eq!(combo("Enter").expect("enter").1.text.as_deref(), Some("\r"));
        assert!(combo("Hyper+X").is_err());
    }
}
