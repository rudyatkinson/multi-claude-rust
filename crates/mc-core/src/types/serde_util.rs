//! Serde helpers that reproduce how the TypeScript version read its values: numbers are numbers
//! whatever their fraction, and a file with a wrong shape in one place falls back to the default
//! there instead of failing as a whole.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// For `x?: true` fields: written only when set.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn one() -> u32 {
    1
}

pub(crate) fn minus_one() -> i64 {
    -1
}

/// Any JSON number. JavaScript has only one number type, and some values the Electron version
/// stored (an `mtimeMs`, a hand-edited setting) carry a fraction where an integer is meant.
#[derive(Deserialize)]
#[serde(untagged)]
enum JsNumber {
    Int(i64),
    Float(f64),
}

impl JsNumber {
    fn truncated(self) -> i64 {
        match self {
            JsNumber::Int(i) => i,
            // saturating; JSON has no NaN or infinity
            JsNumber::Float(f) => f as i64,
        }
    }
}

/// Epoch milliseconds from any JSON number; a fraction is cut off.
pub(crate) fn ms<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    JsNumber::deserialize(d).map(JsNumber::truncated)
}

/// Like [`ms`], for an optional field (`null` counts as missing).
pub(crate) fn opt_ms<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(Option::<JsNumber>::deserialize(d)?.map(JsNumber::truncated))
}

/// A whole number of pixels from any JSON number, rounded as `Math.round` rounds.
pub(crate) fn px<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = match JsNumber::deserialize(d)? {
        JsNumber::Int(i) => i,
        JsNumber::Float(f) => (f + 0.5).floor() as i64,
    };
    Ok(n.clamp(0, i64::from(u32::MAX)) as u32)
}

/// `Array.isArray(raw.x) ? raw.x : []`: anything that is not an array is the default; an array
/// whose items do not fit is an error, so that nothing in it is silently dropped.
pub(crate) fn array_or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(d)?;
    if value.is_array() {
        T::deserialize(value).map_err(D::Error::custom)
    } else {
        Ok(T::default())
    }
}

/// The same for an object: `null`, a list or a scalar is the default.
pub(crate) fn object_or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(d)?;
    if value.is_object() {
        T::deserialize(value).map_err(D::Error::custom)
    } else {
        Ok(T::default())
    }
}

/// A value that is dropped when it does not have the right shape, as a parse that returns null
/// for anything malformed drops it.
pub(crate) fn ok_or_none<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(T::deserialize(value).ok())
}
