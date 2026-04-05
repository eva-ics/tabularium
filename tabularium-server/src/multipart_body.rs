//! Parse `multipart/form-data` from a fully buffered body (curl-friendly).

use std::collections::HashMap;

use axum::body::Bytes;
use futures_util::stream;
use multer::Multipart;
use tabularium::{Error, Result};

/// Collects text fields (UTF-8 lossy) by field name; last duplicate wins.
pub async fn form_fields(body: Bytes, boundary: String) -> Result<HashMap<String, String>> {
    let stream = stream::once(async move { Ok::<Bytes, std::convert::Infallible>(body) });
    let mut multipart = Multipart::new(stream, boundary);
    let mut map = HashMap::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::InvalidInput(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        map.insert(name, String::from_utf8_lossy(&bytes).into_owned());
    }
    Ok(map)
}
