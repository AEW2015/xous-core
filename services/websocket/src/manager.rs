#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

use num_traits::FromPrimitive;

use xous::SID;

#[derive(num_derive::FromPrimitive, num_derive::ToPrimitive, Debug)]
pub enum Opcode {
    Open,
}

pub(crate) fn main(sid: SID) -> ! {
    /* The following statement causes
    PANIC!
Details: PanicInfo { payload: Any { .. }, message: Some(called `Result::unwrap()` on an `Err` value: ()), location: Location { file: "services/websocket/src/manager.rs", line: 14, col: 29 } }
    */
    // log_server::init_wait().unwrap();

    /* The following block causes
        PANIC!
    Details: PanicInfo { payload: Any { .. }, message: Some(failed to forward Opcode::Open: (Envelope { sender: Sender { data: 68943872 }, body: MutableBorrow(MemoryMessage { id: 0, buf: MemoryRange { addr: 140569313546240, size: 4096 }, offset: Some(4), valid: Some(4096) }) }, MemoryInUse)), location: Location { file: "services/websocket/src/main.rs", line: 41, col: 22 } }
    */
/*
    let _ = match log_server::init_wait() {
        Ok(_) => {
            println!("log_server OK ");
            ()
        }
        Err(e) => println!("log_server Error {:#?}", e),
    };
    */
    log::set_max_level(log::LevelFilter::Info);
    log::trace!("my PID is {}", xous::process::id());

    log::info!("ready to accept requests");
    loop {
        let msg = xous::receive_message(sid).unwrap();
        match FromPrimitive::from_usize(msg.body.id()) {
            Some(Opcode::Open) => {
                log::info!("Manager Opcode::Open received");
            }
            None => {}
        }
    }
    // clean up our program
    log::trace!("main loop exit, destroying servers");
    xous::destroy_server(sid).unwrap();
    log::trace!("quitting");
    xous::terminate_process(0)
}
