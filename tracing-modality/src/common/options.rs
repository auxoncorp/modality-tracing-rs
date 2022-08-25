use crate::UserTimelineInfo;
use crate::attr_handlers::AttributeHandler;
use modality_ingest_client::types::AttrVal;
use once_cell::sync::Lazy;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::net::SocketAddr;
use std::thread;

thread_local! {
    static THREAD_TIMELINE_INFO: Lazy<UserTimelineInfo> = Lazy::new(|| {
        let cur = thread::current();
        let name = cur
            .name()
            .map(Into::into)
            .unwrap_or_else(|| format!("thread-{:?}", cur.id()));

        let id = cur.id();

        // So, this is unfortunate, but until https://github.com/rust-lang/rust/issues/67939
        // is decided, there isn't a way to extract the u64 thread ID. So instead, we'll hash
        // it I guess, which also happens to give us a u64. The DefaultHasher uses SipHash,
        // which again is a very silly thing to do to hash the underlying u64, but the
        // alternatives would be:
        //
        // * To write a fake hasher function, that exfiltrates the underlying value
        // * To use a "weaker" hash function than SipHash, such as `rustc_hash::FxHasher`
        //
        // In practice, this only happens once per thread, so the actual impact is negligible,
        // and is acceptable for now, particularly as this is already the hasher used for
        // the standard library's hashmaps, so the code will be present regardless in the
        // final binary. If this CPU usage is impacting your application, please open an issue.
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        let user_id = hasher.finish();

        UserTimelineInfo {
            name,
            user_id,
        }
    });
}

/// Default function used to retrieve the timeline information associated with
/// the current thread. May be replaced by a user-provided callback function.
fn thread_timeline() -> UserTimelineInfo {
    THREAD_TIMELINE_INFO.with(|info| (*info).clone())
}

/// Initialization options.
#[derive(Clone)]
pub struct Options {
    pub(crate) auth: Option<Vec<u8>>,
    pub(crate) metadata: Vec<(String, AttrVal)>,
    pub(crate) server_addr: SocketAddr,
    pub(crate) timeline_identifier: fn() -> UserTimelineInfo,
    pub(crate) lru_cache_size: usize,
    pub(crate) attribute_handlers: Vec<AttributeHandler>,
}

impl Options {
    pub fn new() -> Options {
        let auth = Self::resolve_auth_token();
        let server_addr = ([127, 0, 0, 1], 14182).into();
        Options {
            auth,
            metadata: Vec::new(),
            server_addr,
            timeline_identifier: thread_timeline,
            lru_cache_size: 64,
            attribute_handlers: AttributeHandler::default_handlers(),
        }
    }

    fn resolve_auth_token() -> Option<Vec<u8>> {
        if let Some(from_env) = std::env::var("MODALITY_AUTH_TOKEN")
            .ok()
            .and_then(|t| hex::decode(t).ok())
        {
            return Some(from_env);
        }

        dirs::config_dir()
            .and_then(|config| {
                let file_path = config.join("modality_cli").join(".user_auth_token");
                std::fs::read_to_string(file_path).ok()
            })
            .and_then(|t| hex::decode(t.trim()).ok())
    }

    /// Provide a callback function used to determine the current timeline of execution.
    ///
    /// By default, each operating system thread will be registered as a single timeline.
    /// If a different partitioning of timelines is required, such as one timeline per
    /// async task, or software module within the same thread, the default behavior
    /// can be overridden.
    ///
    /// The callback provided will be called each time a `tracing` event is recorded to
    /// determine the appropriate timeline to record the event on.
    pub fn set_timeline_identifier(&mut self, identifier: fn() -> UserTimelineInfo) {
        self.timeline_identifier = identifier;
    }
    /// A chainable version of [set_timeline_identifier](Self::set_timeline_identifier).
    pub fn with_timeline_identifier(mut self, identifier: fn() -> UserTimelineInfo) -> Self {
        self.timeline_identifier = identifier;
        self
    }

    /// Set an auth token to be provided to modality. Tokens should be a hex stringish value.
    pub fn set_auth<S: AsRef<[u8]>>(&mut self, auth: S) {
        self.auth = hex::decode(auth).ok();
    }
    /// A chainable version of [set_auth](Self::set_auth).
    pub fn with_auth<S: AsRef<[u8]>>(mut self, auth: S) -> Self {
        self.auth = hex::decode(auth).ok();
        self
    }

    /// How many modality timelines should be cached?
    ///
    /// Setting this to a higher number increases the amount of memory used to remember
    /// which `TimelineId`s have been registered with modalityd.
    ///
    /// Setting this to a lower number increases the liklihood that timeline metadata
    /// will need to be re-sent to modalityd (and the amount of network traffic sent)
    pub fn set_lru_cache_size(&mut self, size: usize) {
        self.lru_cache_size = size;
    }
    /// A chainable version of [set_lru_cache_size](Self::set_lru_cache_size).
    pub fn with_lru_cache_size(mut self, size: usize) -> Self {
        self.lru_cache_size = size;
        self
    }

    /// Provide Attribute Handlers to convert from project specific event keys
    /// and values into a Modality compatible format.
    ///
    /// Setting the attribute handlers overrides the default set of attribute
    /// handlers. For more information, see the [`attr_handlers` module](crate::attr_handlers).
    pub fn set_attr_handlers(&mut self, attribute_handlers: Vec<AttributeHandler>) {
        self.attribute_handlers = attribute_handlers;
    }
    /// A chainable version of [set_attr_handlers](Self::set_attr_handlers).
    pub fn with_attr_handlers(mut self, attribute_handlers: Vec<AttributeHandler>) -> Self {
        self.attribute_handlers = attribute_handlers;
        self
    }

    /// Set the name for the root timeline. By default this will be the name of the main thread as
    /// provided by the OS.
    pub fn set_name<S: AsRef<str>>(&mut self, name: S) {
        self.metadata.push((
            "timeline.name".to_string(),
            AttrVal::String(name.as_ref().to_string()),
        ));
    }
    /// A chainable version of [set_name](Self::set_name).
    pub fn with_name<S: AsRef<str>>(mut self, name: S) -> Self {
        self.metadata.push((
            "timeline.name".to_string(),
            AttrVal::String(name.as_ref().to_string()),
        ));
        self
    }

    /// Add arbitrary metadata to the root timeline.
    ///
    /// This can be called multiple times.
    pub fn add_metadata<K: AsRef<str>, V: Into<AttrVal>>(&mut self, key: K, value: V) {
        let key = key.as_ref();
        let key = if key.starts_with("timeline.") {
            key.to_string()
        } else {
            format!("timeline.{}", key)
        };

        self.metadata.push((key, value.into()));
    }
    /// A chainable version of [add_metadata](Self::add_metadata).
    pub fn with_metadata<K: AsRef<str>, V: Into<AttrVal>>(mut self, key: K, value: V) -> Self {
        let key = key.as_ref();
        let key = if key.starts_with("timeline.") {
            key.to_string()
        } else {
            format!("timeline.{}", key)
        };

        self.metadata.push((key, value.into()));
        self
    }

    /// Set the address of modalityd or a modality reflector where trace data should be sent.
    ///
    /// Defaults to `localhost:default_port`
    pub fn set_server_address(&mut self, addr: SocketAddr) {
        self.server_addr = addr;
    }
    /// A chainable version of [set_server_address](Self::set_server_address).
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
