use modality_ingest_client;
use std::io::Read;
use std::net::{SocketAddr, TcpListener};

pub fn modality_null_mock() -> SocketAddr {
    let listener = TcpListener::bind("[::1]:0").expect("bind null mock listener");
    let local_addr = listener.local_addr().unwrap();

    std::thread::spawn(move || {
        for (mut conn, _addr) in listener.accept() {
            std::thread::spawn(move || loop {
                let mut buf = [0u8; 1500];
                if conn.read(&mut buf).is_err() {
                    break;
                }
            });
        }
    });

    local_addr
}

pub struct ModalityNullMock {
    __no_build: (),
}

impl ModalityNullMock {
    pub fn new() -> Self {
        ModalityNullMock { __no_build: () }
    }
}
