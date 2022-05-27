#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

mod api;
mod manager;
use api::Opcode;
use manager::Opcode as ClientOp;
use num_traits::FromPrimitive;
use std::thread;

#[xous::xous_main]
fn xmain() -> ! {
    log_server::init_wait().unwrap();
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    let xns = xous_names::XousNames::new().unwrap();
    let ws_sid = xns
        .register_name(api::SERVER_NAME_WEBSOCKET, None)
        .expect("can't register server");
    log::trace!("registered with NS -- {:?}", ws_sid);
    let _ws_cid = xous::connect(ws_sid).unwrap();

    // get the websocket Manager up and running
    let ws_manager_sid = xous::create_server().expect("couldn't create websocket client server");
    let ws_manager_cid = xous::connect(ws_manager_sid).unwrap();
    thread::spawn({
        move || {
            manager::main(ws_manager_sid);
        }
    });

    log::trace!("ready to accept requests");
    loop {
        let msg = xous::receive_message(ws_sid).unwrap();
        let _response = match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Open) => {
                log::info!("Websocket Opcode::Open");
                let _rsp = match msg
                    .forward(ws_manager_cid, ClientOp::Open as _) {
                        Ok(r) => r,
                        Err(e) => {
                            log::info!("failed to forward Opcode::Open: {:?}", e);
                            panic!();
                        }
                    };
                log::info!("Websocket Opcode::Open complete");
            }
            None => {}
        };
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xns.unregister_server(ws_sid).unwrap();
    xous::destroy_server(ws_sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}
