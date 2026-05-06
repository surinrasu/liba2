mod browser;
mod parser;
mod traits;

pub use browser::ServiceBrowser;
pub use traits::{BrowseEvent, Discovery};

pub const AIRPLAY_SERVICE_TYPE: &str = "_airplay._tcp.local.";

pub const RAOP_SERVICE_TYPE: &str = "_raop._tcp.local.";
