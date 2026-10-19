//! JSON helpers on top of the byte-value API. Available with the `json`
//! feature.

use alloc::string::ToString;

use crate::{ChangeEvent, Error, Hyle, Storage};

impl<St: Storage, Sy: Storage> Hyle<St, Sy> {
    /// Serializes `value` as JSON and stores it under `key`.
    pub fn put_json<T: serde::Serialize>(
        &mut self,
        key: &[u8],
        value: &T,
    ) -> Result<ChangeEvent, Error> {
        let bytes = serde_json::to_vec(value).map_err(|e| Error::Json(e.to_string()))?;
        self.put(key, &bytes)
    }

    /// Reads the JSON stored under `key` and deserializes it into `T`.
    /// `Ok(None)` if the key is absent or deleted.
    pub fn get_json<T: serde::de::DeserializeOwned>(
        &mut self,
        key: &[u8],
    ) -> Result<Option<T>, Error> {
        match self.get_vec(key)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| Error::Json(e.to_string())),
            None => Ok(None),
        }
    }
}
