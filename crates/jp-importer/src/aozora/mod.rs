pub mod index;
pub mod source;

pub use index::{parse_index, AozoraWork};
pub use source::AozoraSource;

use jp_core::{Error, Result};

/// Aozora's webserver hangs on the default reqwest User-Agent and on
/// HTTP/2 negotiation. A browser UA + HTTP/1.1 only gets a snappy 200.
/// Apply the same defaults to aozorahack.org for consistency.
const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

pub(crate) fn aozora_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(BROWSER_UA)
        .http1_only()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Other(format!("build http client: {e}")))
}
