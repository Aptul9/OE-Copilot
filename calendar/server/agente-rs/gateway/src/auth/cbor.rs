//! CBOR quanto serve a WebAuthn (come @levischuck/tiny-cbor): interi, byte, testi, liste, mappe, booleani, null e float, con le lunghezze
//! definite. Si legge il primo elemento e si dice quanti byte occupa: è così che si trova la fine della chiave pubblica dentro authData.

#[derive(Clone, Debug, PartialEq)]
pub enum Cbor {
    Intero(i128),
    Byte(Vec<u8>),
    Testo(String),
    Lista(Vec<Cbor>),
    Mappa(Vec<(Cbor, Cbor)>),
    Booleano(bool),
    Nullo,
    Indefinito,
    Decimale(f64),
}

impl Cbor {
    /// Il valore di una chiave di una mappa (Map.get), None se non è una mappa o la chiave non c'è.
    pub fn prendi(&self, chiave: &Cbor) -> Option<&Cbor> {
        match self {
            Cbor::Mappa(v) => v.iter().find(|(k, _)| k == chiave).map(|(_, x)| x),
            _ => None,
        }
    }

    pub fn prendi_testo(&self, chiave: &str) -> Option<&Cbor> {
        self.prendi(&Cbor::Testo(chiave.to_string()))
    }

    pub fn prendi_intero(&self, chiave: i128) -> Option<&Cbor> {
        self.prendi(&Cbor::Intero(chiave))
    }

    pub fn intero(&self) -> Option<i128> {
        match self {
            Cbor::Intero(n) => Some(*n),
            _ => None,
        }
    }

    pub fn byte(&self) -> Option<&[u8]> {
        match self {
            Cbor::Byte(b) => Some(b),
            _ => None,
        }
    }
}

const PROFONDITA_MAX: usize = 64;

fn lunghezza(d: &[u8], i: &mut usize, info: u8) -> Result<u64, String> {
    let mut leggi = |n: usize| -> Result<u64, String> {
        let pezzo = d.get(*i..*i + n).ok_or("CBOR troncato")?;
        *i += n;
        Ok(pezzo.iter().fold(0u64, |a, b| (a << 8) | u64::from(*b)))
    };
    match info {
        0..=23 => Ok(u64::from(info)),
        24 => leggi(1),
        25 => leggi(2),
        26 => leggi(4),
        27 => leggi(8),
        _ => Err("lunghezza CBOR non supportata".to_string()),
    }
}

fn mezza(h: u16) -> f64 {
    let esponente = (h >> 10) & 0x1f;
    let mantissa = f64::from(h & 0x3ff);
    let segno = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    segno
        * match esponente {
            0 => mantissa * 2f64.powi(-24),
            31 => {
                if mantissa == 0.0 {
                    f64::INFINITY
                } else {
                    f64::NAN
                }
            }
            e => (1.0 + mantissa / 1024.0) * 2f64.powi(i32::from(e) - 15),
        }
}

fn elemento(d: &[u8], i: &mut usize, livello: usize) -> Result<Cbor, String> {
    if livello > PROFONDITA_MAX {
        return Err("CBOR troppo annidato".to_string());
    }
    let primo = *d.get(*i).ok_or("CBOR troncato")?;
    *i += 1;
    let (maggiore, info) = (primo >> 5, primo & 31);
    match maggiore {
        0 => Ok(Cbor::Intero(i128::from(lunghezza(d, i, info)?))),
        1 => Ok(Cbor::Intero(-1 - i128::from(lunghezza(d, i, info)?))),
        2 | 3 => {
            let n = usize::try_from(lunghezza(d, i, info)?).map_err(|_| "CBOR troppo lungo")?;
            let pezzo = d.get(*i..i.checked_add(n).ok_or("CBOR troppo lungo")?).ok_or("CBOR troncato")?.to_vec();
            *i += n;
            if maggiore == 2 {
                Ok(Cbor::Byte(pezzo))
            } else {
                String::from_utf8(pezzo).map(Cbor::Testo).map_err(|_| "testo CBOR non UTF-8".to_string())
            }
        }
        4 => {
            let n = lunghezza(d, i, info)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(elemento(d, i, livello + 1)?);
            }
            Ok(Cbor::Lista(v))
        }
        5 => {
            let n = lunghezza(d, i, info)?;
            let mut v = Vec::new();
            for _ in 0..n {
                let k = elemento(d, i, livello + 1)?;
                let x = elemento(d, i, livello + 1)?;
                v.push((k, x));
            }
            Ok(Cbor::Mappa(v))
        }
        6 => {
            // un'etichetta: si legge il valore che porta
            lunghezza(d, i, info)?;
            elemento(d, i, livello + 1)
        }
        _ => match info {
            20 => Ok(Cbor::Booleano(false)),
            21 => Ok(Cbor::Booleano(true)),
            22 => Ok(Cbor::Nullo),
            23 => Ok(Cbor::Indefinito),
            25 => Ok(Cbor::Decimale(mezza(lunghezza(d, i, 25)? as u16))),
            26 => Ok(Cbor::Decimale(f64::from(f32::from_bits(lunghezza(d, i, 26)? as u32)))),
            27 => Ok(Cbor::Decimale(f64::from_bits(lunghezza(d, i, 27)?))),
            _ => Err("valore CBOR non supportato".to_string()),
        },
    }
}

/// Il primo elemento e quanti byte occupa.
pub fn primo(d: &[u8]) -> Result<(Cbor, usize), String> {
    let mut i = 0;
    let v = elemento(d, &mut i, 0)?;
    Ok((v, i))
}
