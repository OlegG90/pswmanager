//! Random passwords from the OS random number generator.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!#$%&()*+,-./:;<=>?@[]^_{|}~";
/// Characters that are easy to mistake for one another.
const LOOK_ALIKES: &str = "Il1|O0o";
pub const LENGTH_RANGE: (usize, usize) = (8, 64);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub length: usize,
    pub upper: bool,
    pub lower: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_look_alikes: bool,
}

/// A password with at least one character of every chosen kind.
pub fn generate(options: &Options) -> Result<Zeroizing<String>, String> {
    let (min, max) = LENGTH_RANGE;
    let length = options.length.clamp(min, max);
    let sets: Vec<Vec<char>> = [
        (options.upper, UPPER),
        (options.lower, LOWER),
        (options.digits, DIGITS),
        (options.symbols, SYMBOLS),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, set)| set.chars().filter(|c| !(options.exclude_look_alikes && LOOK_ALIKES.contains(*c))).collect())
    .collect();
    if sets.is_empty() {
        return Err("Choose at least one kind of character".into());
    }
    let all: Vec<char> = sets.concat();
    let mut chars = sets.iter().map(|set| Ok(set[random_below(set.len())?])).collect::<Result<Vec<char>, String>>()?;
    while chars.len() < length {
        chars.push(all[random_below(all.len())?]);
    }
    // Shuffle so the guaranteed characters are not always at the start.
    for i in (1..chars.len()).rev() {
        chars.swap(i, random_below(i + 1)?);
    }
    Ok(Zeroizing::new(chars.into_iter().collect()))
}

/// A uniform random number in `0..n`, without modulo bias.
fn random_below(n: usize) -> Result<usize, String> {
    let n = n as u32;
    let limit = u32::MAX - u32::MAX % n;
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let value = u32::from_le_bytes(bytes);
        if value < limit {
            return Ok((value % n) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(length: usize) -> Options {
        Options { length, upper: true, lower: true, digits: true, symbols: false, exclude_look_alikes: false }
    }

    #[test]
    fn every_chosen_kind_appears() {
        for _ in 0..200 {
            let password = generate(&options(8)).unwrap();
            assert_eq!(password.chars().count(), 8);
            assert!(password.chars().any(|c| c.is_ascii_uppercase()));
            assert!(password.chars().any(|c| c.is_ascii_lowercase()));
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn look_alikes_can_be_left_out() {
        let opts = Options { exclude_look_alikes: true, symbols: true, ..options(64) };
        for _ in 0..50 {
            assert!(!generate(&opts).unwrap().chars().any(|c| LOOK_ALIKES.contains(c)));
        }
    }

    #[test]
    fn length_is_kept_within_limits() {
        assert_eq!(generate(&options(3)).unwrap().len(), 8);
        assert_eq!(generate(&options(500)).unwrap().len(), 64);
        let nothing = Options { upper: false, lower: false, digits: false, ..options(20) };
        assert!(generate(&nothing).is_err());
    }

    #[test]
    fn passwords_differ() {
        assert_ne!(*generate(&options(20)).unwrap(), *generate(&options(20)).unwrap());
    }
}
