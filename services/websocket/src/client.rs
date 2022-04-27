#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod poll;
use poll::*;

use derive_deref::*;
use embedded_websocket as ws;
use num_traits::{FromPrimitive, ToPrimitive};
use rand::rngs::ThreadRng;
use rustls::{ClientConnection, StreamOwned};
use rustls_connector::*;
use std::num::NonZeroU8;
use std::{
    collections::HashMap,
    convert::TryInto,
    io::{Error, ErrorKind, Read, Write},
    net::TcpStream,
    thread,
};
use url::Url;
use ws::framer::{Framer, FramerError};
use ws::WebSocketCloseStatusCode as StatusCode;
use ws::WebSocketSendMessageType as MessageType;
use ws::{WebSocketClient, WebSocketOptions, WebSocketState};
use xous::CID;
use xous_ipc::Buffer;

use std::time::Duration;

/** time between reglar websocket keep-alive requests */
pub(crate) const KEEPALIVE_TIMEOUT_SECONDS: Duration = Duration::from_secs(55);
pub(crate) const HINT_LEN: usize = 128;
/** limit on the byte length of certificate authority strings */
/*
 A websocket header requires at least 14 bytes of the websocket buffer
 ( see https://crates.io/crates/embedded-websocket ) leaving the remainder
 available for the payload. This relates directly to the frame buffer.
 There may be advantage in independently specifying the read, frame, and write buffer sizes.
 TODO review/test/optimise WEBSOCKET_BUFFER_LEN
*/
pub(crate) const WEBSOCKET_BUFFER_LEN: usize = 4096;
pub(crate) const WEBSOCKET_PAYLOAD_LEN: usize = 4080;

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Frame {
    pub bytes: [u8; WEBSOCKET_PAYLOAD_LEN],
}

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    /// Close an existing websocket.
    /// xous::Message::new_scalar(Opcode::Close, _, _, _, _)
    Close = 1,
    /// send a websocket frame
    Send,
    /// Return the current State of the websocket
    /// 1=Open, 0=notOpen
    /// xous::Message::new_scalar(Opcode::State, _, _, _, _)
    State,
    /// Send a KeepAliveRequest.
    /// An independent background thread is spawned to pump a regular Tick (KEEPALIVE_TIMEOUT_SECONDS)
    /// so there is normally no need to call this Opcode.
    /// xous::Message::new_scalar(Opcode::Tick, _, _, _, _)
    Tick,
    /// Close all websockets and shutdown server
    /// xous::Message::new_scalar(Opcode::Quit, _, _, _, _)
    Quit,
}

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug, PartialEq)]
pub(crate) enum WsError {
    /// This Opcode accepts Scalar calls
    Scalar,
    /// This Opcode accepts Blocking Scalar calls
    ScalarBlock,
    /// This Opcode accepts Memory calls
    Memory,
    /// This Opcode accepts Blocking Memory calls
    MemoryBlock,
    /// Websocket assets corruption
    AssetsFault,
    /// Error in Websocket protocol
    ProtocolError,
}


struct Client<R: rand::RngCore> {
    /** the configuration of an open websocket */
    socket: WebSocketClient<R>,
    /** a websocket stream when opened on a tls connection */
    wss_stream: Option<WsStream<StreamOwned<ClientConnection, TcpStream>>>,
    /** a websocket stream when opened on a tcp connection */
    ws_stream: Option<WsStream<TcpStream>>,
    /** the underlying tcp stream */
    tcp_stream: TcpStream,
    /** the framer read buffer */
    read_buf: [u8; WEBSOCKET_BUFFER_LEN],
    /** the framer read cursor */
    read_cursor: usize,
    /** the framer write buffer */
    write_buf: [u8; WEBSOCKET_BUFFER_LEN],
    /** the callback_id to use when relaying an inbound websocket frame */
    cid: CID,
    /** the opcode to use when relaying an inbound websocket frame */
    opcode: u32,
}

impl<R: rand::RngCore> Client<R> {
    pub(crate) fn new(
        &mut self,
        path: &'a str,
        host: &'a str,
        origin: &'a str,
        sub_protocols: Option<&'a [&'a str]>,
        additional_headers: Option<&'a [&'a str]>,
    ) {
        let websocket_options = WebSocketOptions {
            path,
            host,
            origin,
            sub_protocols,
            additional_headers,
        };
        self.read_buf = [0; WEBSOCKET_BUFFER_LEN];
        self.read_cursor = 0;
        self.write_buf = [0; WEBSOCKET_BUFFER_LEN];

        let mut ws_client = WebSocketClient::new_client(rand::thread_rng());
        let mut framer = Framer::new(
            &mut self.read_buf,
            &mut self.read_cursor,
            &mut self.write_buf,
            &mut ws_client,
        );

        log::trace!("Will start websocket at {:?}", url.host_str().unwrap());
        // Create a TCP Stream between this device and the remote Server
        let target = format!("{}:{}", url.host_str().unwrap(), url.port().unwrap());
        log::info!("Opening TCP connection to {:?}", target);
        self.tcp_stream = match TcpStream::connect(&target) {
            Ok(tcp_stream) => tcp_stream,
            Err(e) => {
                let hint = format!("Failed to open TCP Stream {:?}", e);
                buf.replace(drop(&hint)).expect("failed replace buffer");
                continue;
            }
        };

        log::info!("TCP connected to {:?}", target);

        self.ws_stream = None;
        self.wss_stream = None;
        let tcp_clone = match self.tcp_stream.try_clone() {
            Ok(c) => c,
            Err(e) => {
                let hint = format!("Failed to clone TCP Stream {:?}", e);
                buf.replace(drop(&hint)).expect("failed replace buffer");
                continue;
            }
        };
        let sub_protocol: xous_ipc::String<SUB_PROTOCOL_LEN>;
        if ws_config.certificate_authority.is_none() {
            // Initiate a websocket opening handshake over the TCP Stream
            let mut stream = WsStream(self.tcp_stream);
            sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                Ok(opt) => match opt {
                    Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                    None => xous_ipc::String::from_str(""),
                },
                Err(e) => {
                    let hint = format!("Unable to connect WebSocket {:?}", e);
                    buf.replace(drop(&hint)).expect("failed replace buffer");
                    continue;
                }
            };
            self.ws_stream = Some(stream);
        } else {
            // Create a TLS connection to the remote Server on the TCP Stream
            let ca = ws_config.certificate_authority.unwrap();
            let ca = ca
                .as_str()
                .expect("certificate_authority utf-8 decode error");
            let tls_connector = RustlsConnector::from(Self.ssl_config(ca));
            self.tls_stream = match tls_connector.connect(url.host_str().unwrap(), tcp_stream) {
                Ok(tls_stream) => {
                    log::info!("TLS connected to {:?}", url.host_str().unwrap());
                    tls_stream
                }
                Err(e) => {
                    let hint = format!("Failed to complete TLS handshake {:?}", e);
                    buf.replace(drop(&hint)).expect("failed replace buffer");
                    continue;
                }
            };
            // Initiate a websocket opening handshake over the TLS Stream
            let mut stream = WsStream(self.tls_stream);
            sub_protocol = match framer.connect(&mut stream, &websocket_options) {
                Ok(opt) => match opt {
                    Some(sp) => xous_ipc::String::from_str(sp.to_string()),
                    None => xous_ipc::String::from_str(""),
                },
                Err(e) => {
                    let hint = format!("Unable to connect WebSocket {:?}", e);
                    buf.replace(drop(&hint)).expect("failed replace buffer");
                    continue;
                }
            };
            self.wss_stream = Some(stream);
        }

        let mut response = api::Return::SubProtocol(sub_protocol);
        match framer.state() {
            WebSocketState::Open => {
                log::info!("WebSocket connected with protocol: {:?}", sub_protocol);
                
                // start a regular poll of the websocket for inbound frames
                
                let mut poll = Poll::new((
                            ws_config.cid,
                            ws_config.opcode,
                            tcp_clone,
                            ws_stream,
                            wss_stream,
                            ws_client,
                        );
                
                thread::spawn({
                    move || {
                        poll.main();
                    }
                });

                // start the main loop of the Websocket Client
                self.main();
            }
            _ => {
                let hint = format!("WebSocket failed to connect {:?}", framer.state());
                response = drop(&hint);
            }
        }
    }


    fn main() -> ! {
        log_server::init_wait().unwrap();
        log::set_max_level(log::LevelFilter::Info);
        log::info!("my PID is {}", xous::process::id());

        let xns = xous_names::XousNames::new().unwrap();
        let ws_sid = xns
            .register_name(api::SERVER_NAME_WEBSOCKET, None)
            .expect("can't register server");
        log::trace!("registered with NS -- {:?}", ws_sid);
        let ws_cid = xous::connect(ws_sid).unwrap();

        // build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
        spawn_tick_pump(ws_cid);

        /* holds the assets of existing websockets by pid - and as such - limits each pid to 1 websocket. */
        // TODO review the limitation of 1 websocket per pid.
        let mut store: HashMap<NonZeroU8, Assets<ThreadRng>> = HashMap::new();

        log::trace!("ready to accept requests");
        loop {
            let mut msg = xous::receive_message(ws_sid).unwrap();
            match FromPrimitive::from_usize(msg.body.id()) {
                Some(Opcode::Close) => {
                    log::info!("Websocket Opcode::Close");
                    if !validate_msg(&mut msg, WsError::Scalar, Opcode::Close) {
                        continue;
                    }
                    let pid = msg.sender.pid().unwrap();
                    let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                    let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                        Some(assets) => {
                            framer = Framer::new(
                                &mut assets.read_buf[..],
                                &mut assets.read_cursor,
                                &mut assets.write_buf[..],
                                &mut assets.socket,
                            );
                            (&mut assets.wss_stream, &mut assets.ws_stream)
                        }
                        None => {
                            log::warn!("Websocket assets not in list");
                            xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                            continue;
                        }
                    };

                    let response = match wss_stream {
                        Some(stream) => framer.close(&mut *stream, StatusCode::NormalClosure, None),
                        None => match ws_stream {
                            Some(stream) => {
                                framer.close(&mut *stream, StatusCode::NormalClosure, None)
                            }
                            None => {
                                log::warn!("Assets missing both wss_stream and ws_stream");
                                xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                                continue;
                            }
                        },
                    };

                    match response {
                        Ok(()) => log::info!("Sent close handshake"),
                        Err(e) => {
                            log::warn!("Failed to send close handshake {:?}", e);
                            xous::return_scalar(msg.sender, WsError::ProtocolError as usize).ok();
                            continue;
                        }
                    };
                    log::info!("Websocket Opcode::Close complete");
                }
                Some(Opcode::Send) => {
                    if !validate_msg(&mut msg, WsError::Memory, Opcode::Send) {
                        continue;
                    }
                    log::info!("Websocket Opcode::Send");
                    let pid = msg.sender.pid().unwrap();
                    let mut buf = unsafe {
                        Buffer::from_memory_message_mut(msg.body.memory_message_mut().unwrap())
                    };

                    let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                    let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                        Some(assets) => {
                            framer = Framer::new(
                                &mut assets.read_buf[..],
                                &mut assets.read_cursor,
                                &mut assets.write_buf[..],
                                &mut assets.socket,
                            );
                            (&mut assets.wss_stream, &mut assets.ws_stream)
                        }
                        None => {
                            log::info!("Websocket assets not in list");
                            continue;
                        }
                    };
                    match framer.state() {
                        WebSocketState::Open => {}
                        _ => {
                            let hint = format!("WebSocket DOA {:?}", framer.state());
                            buf.replace(drop(&hint)).expect("failed replace buffer");
                            continue;
                        }
                    }

                    let response = match wss_stream {
                        Some(stream) => write(&mut framer, &mut *stream, &buf),
                        None => match ws_stream {
                            Some(stream) => write(&mut framer, &mut *stream, &buf),
                            None => {
                                log::warn!("Assets missing both wss_stream and ws_stream");
                                continue;
                            }
                        },
                    };
                    match response {
                        Ok(()) => log::info!("Websocket frame sent"),
                        Err(e) => {
                            let hint = format!("failed to send Websocket frame {:?}", e);
                            buf.replace(drop(&hint)).expect("failed replace buffer");
                            continue;
                        }
                    };
                    log::info!("Websocket Opcode::Send complete");
                }
                Some(Opcode::State) => {
                    log::info!("Websocket Opcode::State");
                    if !validate_msg(&mut msg, WsError::ScalarBlock, Opcode::State) {
                        continue;
                    }
                    let pid = msg.sender.pid().unwrap();
                    match store.get_mut(&pid) {
                        Some(assets) => {
                            let framer = Framer::new(
                                &mut assets.read_buf,
                                &mut assets.read_cursor,
                                &mut assets.write_buf,
                                &mut assets.socket,
                            );

                            if framer.state() == WebSocketState::Open {
                                xous::return_scalar(msg.sender, 1)
                                    .expect("failed to return WebSocketState");
                            }
                        }
                        None => xous::return_scalar(msg.sender, 0)
                            .expect("failed to return WebSocketState"),
                    };
                    log::info!("Websocket Opcode::State complete");
                }
                Some(Opcode::Tick) => {
                    log::info!("Websocket Opcode::Tick");
                    if !validate_msg(&mut msg, WsError::Scalar, Opcode::Tick) {
                        continue;
                    }
                    let pid = msg.sender.pid().unwrap();
                    let mut framer: Framer<rand::rngs::ThreadRng, embedded_websocket::Client>;
                    let (wss_stream, ws_stream) = match store.get_mut(&pid) {
                        Some(assets) => {
                            framer = Framer::new(
                                &mut assets.read_buf[..],
                                &mut assets.read_cursor,
                                &mut assets.write_buf[..],
                                &mut assets.socket,
                            );
                            (&mut assets.wss_stream, &mut assets.ws_stream)
                        }
                        None => {
                            log::warn!("Websocket assets not in list");
                            xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                            continue;
                        }
                    };

                    // TODO review keep alive request technique
                    let frame_buf = "keep alive please :-)".as_bytes();

                    let response = match wss_stream {
                        Some(stream) => {
                            framer.write(&mut *stream, MessageType::Text, true, &frame_buf)
                        }

                        None => match ws_stream {
                            Some(stream) => {
                                framer.write(&mut *stream, MessageType::Text, true, &frame_buf)
                            }

                            None => {
                                log::warn!("Assets missing both wss_stream and ws_stream");
                                xous::return_scalar(msg.sender, WsError::AssetsFault as usize).ok();
                                continue;
                            }
                        },
                    };

                    match response {
                        Ok(()) => log::info!("Websocket keep-alive request sent"),
                        Err(e) => {
                            log::info!("failed to send Websocket keep-alive request {:?}", e);
                            continue;
                        }
                    };

                    log::info!("Websocket Opcode::Tick complete");
                }

                Some(Opcode::Quit) => {
                    log::warn!("Websocket Opcode::Quit");
                    if !validate_msg(&mut msg, WsError::Scalar, Opcode::Quit) {
                        continue;
                    }
                    let close_op = Opcode::Close.to_usize().unwrap();
                    for (_pid, assets) in &mut store {
                        xous::send_message(
                            assets.cid,
                            xous::Message::new_scalar(close_op, 0, 0, 0, 0),
                        )
                        .expect("couldn't send Websocket poll");
                    }
                    log::warn!("Websocket Opcode::Quit complete");
                    break;
                }
                None => {
                    log::error!("couldn't convert opcode: {:?}", msg);
                }
            }
        }
        // clean up our program
        log::trace!("main loop exit, destroying servers");
        xns.unregister_server(ws_sid).unwrap();
        xous::destroy_server(ws_sid).unwrap();
        log::trace!("quitting");
        xous::terminate_process(0)
    }



    /** complete the machinations of setting up a rustls::ClientConfig */
    fn ssl_config(certificate_authority: &str) -> rustls::ClientConfig {
        let mut cert_bytes = std::io::Cursor::new(&certificate_authority);
        let roots = rustls_pemfile::certs(&mut cert_bytes).expect("parseable PEM files");
        let roots = roots.iter().map(|v| rustls::Certificate(v.clone()));

        let mut root_certs = rustls::RootCertStore::empty();
        for root in roots {
            root_certs.add(&root).unwrap();
        }

        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_certs)
            .with_no_client_auth()
    }
}

// build a thread that emits a regular WebSocketOp::Tick to send a KeepAliveRequest
fn spawn_tick_pump(cid: CID) {
    thread::spawn({
        move || {
            let tt = ticktimer_server::Ticktimer::new().unwrap();
            loop {
                tt.sleep_ms(KEEPALIVE_TIMEOUT_SECONDS.as_millis().try_into().unwrap())
                    .unwrap();
                xous::send_message(
                    cid,
                    xous::Message::new_scalar(
                        Opcode::Tick.to_usize().unwrap(),
                        KEEPALIVE_TIMEOUT_SECONDS.as_secs().try_into().unwrap(),
                        0,
                        0,
                        0,
                    ),
                )
                .expect("couldn't send Websocket tick");
            }
        }
    });
}



fn write<E, R, S, T>(
    framer: &mut Framer<R, S>,
    stream: &mut T,
    buffer: &[u8],
) -> Result<(), FramerError<E>>
where
    E: std::fmt::Debug,
    R: rand::RngCore,
    T: ws::framer::Stream<E>,
    S: ws::WebSocketType,
{
    let mut ret = Ok(());
    let mut end_of_message = false;
    let mut start = 0;
    let mut slice;
    while !end_of_message {
        log::info!("start = {:?}", start);
        if buffer.len() < (start + WEBSOCKET_PAYLOAD_LEN) {
            end_of_message = true;
            slice = &buffer[start..];
        } else {
            slice = &buffer[start..(start + WEBSOCKET_PAYLOAD_LEN)];
        }
        ret = framer.write(&mut *stream, MessageType::Binary, end_of_message, slice);
        start = start + WEBSOCKET_PAYLOAD_LEN;
    }
    ret
}


