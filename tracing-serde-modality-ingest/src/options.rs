use std::net::SocketAddr;

pub struct Options {
    pub(crate) auth: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) server_addr: SocketAddr,
}

impl Options {
    pub fn new() -> Options {
        let auth = std::env::var("MODALITY_LICENSE_KEY").ok();
        let server_addr = ([127, 0, 0, 1], 14182).into();
        Options {
            name: None,
            auth,
            server_addr,
        }
    }

    pub fn set_auth(&mut self, auth: String) {
        self.auth = Some(auth);
    }
    pub fn with_auth(mut self, auth: String) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn set_name(&mut self, auth: String) {
        self.auth = Some(auth);
    }
    pub fn with_name(mut self, auth: String) -> Self {
        self.auth = Some(auth);
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
