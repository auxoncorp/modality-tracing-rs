use modality_ingest_protocol::types::AttrVal;
use once_cell::sync::Lazy;
use std::net::SocketAddr;
use std::sync::RwLock;

/// This is here to facilitate configuration of multi-threaded tracing subscribers.
/// It is an implementation detail, and may be removed at any time. Changing or
/// removal of this static will NOT be considered a breaking change.
#[doc(hidden)]
pub static GLOBAL_OPTIONS: Lazy<RwLock<Options>> = Lazy::new(|| RwLock::new(Options::default()));

#[derive(Clone)]
pub struct Options {
    pub(crate) auth: Option<String>,
    pub(crate) metadata: Vec<(String, AttrVal)>,
    pub(crate) server_addr: SocketAddr,
}

impl Options {
    pub fn new() -> Options {
        let auth = std::env::var("MODALITY_LICENSE_KEY").ok();
        let server_addr = ([127, 0, 0, 1], 14182).into();
        Options {
            auth,
            metadata: Vec::new(),
            server_addr,
        }
    }

    pub fn set_auth<S: AsRef<str>>(&mut self, auth: S) {
        self.auth = Some(auth.as_ref().to_string());
    }
    pub fn with_auth<S: AsRef<str>>(mut self, auth: S) -> Self {
        self.auth = Some(auth.as_ref().to_string());
        self
    }

    pub fn set_name<S: AsRef<str>>(&mut self, name: S) {
        self.metadata.push((
            "timeline.name".to_string(),
            AttrVal::String(name.as_ref().to_string()),
        ));
    }
    pub fn with_name<S: AsRef<str>>(mut self, name: S) -> Self {
        self.metadata.push((
            "timeline.name".to_string(),
            AttrVal::String(name.as_ref().to_string()),
        ));
        self
    }

    pub fn add_metadata<K: AsRef<str>>(&mut self, key: K, value: AttrVal) {
        self.metadata
            .push((format!("timeline.{}", key.as_ref()), value));
    }
    pub fn with_metadata<K: AsRef<str>>(mut self, key: K, value: AttrVal) -> Self {
        self.metadata
            .push((format!("timeline.{}", key.as_ref()), value));
        self
    }

    pub fn set_server_address(&mut self, addr: SocketAddr) {
        self.server_addr = addr;
    }
    pub fn with_server_address(mut self, addr: SocketAddr) -> Self {
        self.server_addr = addr;
        self
    }
}

impl Default for Options {
    fn default() -> Options {
        Options::new()
    }
}
