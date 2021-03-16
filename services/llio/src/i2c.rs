use heapless::Vec;
use heapless::consts::*;

use llio::api::*;
use llio::send_i2c_response;
use utralib::*;

#[derive(Eq, PartialEq)]
enum I2cState {
    Idle,
    Write,
    Read,
}
pub struct I2cStateMachine {
    transaction: I2cTransaction,
    state: I2cState,
    index: u32,  // index of the current buffer in the state machine
    timestamp: u64, // timestamp of the last transaction
    ticktimer: xous::CID, // a connection to the ticktimer so we can measure timeouts
    i2c_csr: utralib::CSR<u32>,
    listeners: Vec<xous::CID, U32>,
}
impl I2cStateMachine {
    pub fn new(ticktimer: xous::CID, i2c_base: *mut u32) -> Self {
        I2cStateMachine {
            transaction: I2cTransaction::new(),
            state: I2cState::Idle,
            timestamp: ticktimer_server::elapsed_ms(ticktimer).unwrap(),
            ticktimer,
            i2c_csr: CSR::new(i2c_base),
            index: 0,
            listeners: Vec::new(),
        }
    }
    pub fn initiate(&mut self, transaction: I2cTransaction ) -> I2cStatus {
        // sanity-check the bounds limits, currently imposed by trait implementations available in rkyv
        if transaction.txlen > 31 || transaction.rxlen > 31 {
            return I2cStatus::ResponseFormatError
        }

        let now = ticktimer_server::elapsed_ms(self.ticktimer).unwrap();
        if self.state != I2cState::Idle && ((now - self.timestamp) < self.transaction.timeout_ms as u64) {
            // we're in a transaction that hadn't timed out, can't accept a new one
            I2cStatus::ResponseBusy
        } else {
            if self.state != I2cState::Idle {
                // we're in a transaction, but previous transaction had timed out...
                self.report_timeout();
                // reset our state parameter
                self.state = I2cState::Idle;
                self.index = 0;
                // now we're ready to move on and try a new transaction. We hope! Maybe the block should be reset?? TBD. Need to understand the nature of the timeouts better, if and when they do happen.
            }
            self.timestamp = now;
            self.transaction = transaction.clone();

            if self.transaction.status == I2cStatus::RequestIncoming {
                self.transaction.status = I2cStatus::ResponseInProgress;
                // now do the BusAddr stuff, so that the we can get the irq response
                if let Some(_txbuf) = self.transaction.txbuf {
                    // initiate bus address with write bit set
                    self.state = I2cState::Write;
                    self.i2c_csr.wfo(utra::i2c::TXR_TXR, (self.transaction.bus_addr << 1 | 0) as u32);
                    self.index = 0;
                    self.i2c_csr.wo(utra::i2c::COMMAND,
                        self.i2c_csr.ms(utra::i2c::COMMAND_WR, 1) |
                        self.i2c_csr.ms(utra::i2c::COMMAND_STA, 1)
                    );
                    I2cStatus::ResponseInProgress
                } else if let Some(_rxbuf) = self.transaction.rxbuf {
                    // initiate bus address with read bit set
                    self.state = I2cState::Read;
                    self.i2c_csr.wfo(utra::i2c::TXR_TXR, (self.transaction.bus_addr << 1 | 1) as u32);
                    self.index = 0;
                    self.i2c_csr.wo(utra::i2c::COMMAND,
                        self.i2c_csr.ms(utra::i2c::COMMAND_WR, 1) |
                        self.i2c_csr.ms(utra::i2c::COMMAND_STA, 1)
                    );
                    I2cStatus::ResponseInProgress
                } else {
                    // no buffers specified, erase everything and go to idle
                    self.state = I2cState::Idle;
                    self.transaction = I2cTransaction::new();
                    I2cStatus::ResponseFormatError
                }
            } else {
                I2cStatus::ResponseFormatError  // the status field was not formatted correctly to accept the transaction
            }
        }
    }
    fn report_nack(&mut self) {
        // report the NACK situation to all the listeners
        let mut nack = I2cTransaction::new();
        nack.status = I2cStatus::ResponseNack;
        for &listener in self.listeners.iter() {
            send_i2c_response(listener, nack).expect("LLIO|I2C: couldn't send NACK to listeners");
        }
    }
    fn report_timeout(&mut self) {
        let mut timeout = I2cTransaction::new();
        timeout.status = I2cStatus::ResponseTimeout;
        for &listener in self.listeners.iter() {
            send_i2c_response(listener, timeout).expect("LLIO|I2c: couldn't send timeout error to liseners");
        }
    }
    fn report_write_done(&mut self) {
        // report the end of a write-only transaction to all the listeners
        let mut ack = I2cTransaction::new();
        ack.status = I2cStatus::ResponseWriteOk;
        for &listener in self.listeners.iter() {
            send_i2c_response(listener, ack).expect("LLIO|I2C: couldn't send write ACK to listeners");
        }
    }
    fn report_read_done(&mut self) {
        // report the result of a read transaction to all the listeners
        self.transaction.status = I2cStatus::ResponseReadOk;
        for &listener in self.listeners.iter() {
            send_i2c_response(listener, self.transaction).expect("LLIO|I2C: couldn't send read response to listeners");
        }
    }
    pub fn register_listener(&mut self, listener: xous::CID) -> Result<(), xous::CID> {
        self.listeners.push(listener)
    }
    pub fn handler(&mut self) {
        // check if the transaction had actually timed out
        let now = ticktimer_server::elapsed_ms(self.ticktimer).unwrap();
        if now - self.timestamp > self.transaction.timeout_ms as u64 {
            // previous transaction had timed out...
            self.report_timeout();
            // reset our state parameter
            self.state = I2cState::Idle;
            self.index = 0;
            self.timestamp = now;
            return;
        }
        self.timestamp = now;

        match self.state {
            I2cState::Write => {
                if let Some(txbuf) = self.transaction.txbuf {
                    // check ack bit
                    if self.i2c_csr.rf(utra::i2c::STATUS_RXACK) != 1 {
                        self.state = I2cState::Idle;
                        self.transaction = I2cTransaction::new();
                        self.report_nack();
                    }
                    // send next byte if there is one
                    if self.index < self.transaction.txlen {
                        self.i2c_csr.wfo(utra::i2c::TXR_TXR, txbuf[self.index as usize] as u32);
                        if self.index == (self.transaction.txlen - 1) && self.transaction.rxbuf.is_none() {
                            // send a stop bit if this is the very last in the series
                            self.i2c_csr.wo(utra::i2c::COMMAND,
                                self.i2c_csr.ms(utra::i2c::COMMAND_WR, 1) |
                                self.i2c_csr.ms(utra::i2c::COMMAND_STO, 1)
                            );
                        } else {
                            self.i2c_csr.wfo(utra::i2c::COMMAND_WR, 1);
                        }
                        self.index += 1;
                    } else {
                        if let Some(_rxbuf) = self.transaction.rxbuf {
                            // initiate bus address with read bit set
                            self.state = I2cState::Read;
                            self.i2c_csr.wfo(utra::i2c::TXR_TXR, (self.transaction.bus_addr << 1 | 1) as u32);
                            self.index = 0;
                            self.i2c_csr.wo(utra::i2c::COMMAND,
                                self.i2c_csr.ms(utra::i2c::COMMAND_WR, 1) |
                                self.i2c_csr.ms(utra::i2c::COMMAND_STA, 1)
                            );
                        } else {
                            self.report_write_done();
                            self.state = I2cState::Idle;
                        }
                    }
                } else {
                    // we should never get here, because txbuf was checked as Some() by the setup routine
                    log::error!("LLIO|I2C: illegal write state");
                }
            },
            I2cState::Read => {
                if let Some(mut rxbuf) = self.transaction.rxbuf {
                    if self.index > 0 {
                        // we are re-entering from a previous call, store the read value from the previous call
                        rxbuf[self.index as usize - 1] = self.i2c_csr.rf(utra::i2c::RXR_RXR) as u8;
                    }
                    if self.index < self.transaction.rxlen {
                        if self.index == (self.transaction.rxlen - 1) {
                            self.i2c_csr.wo(utra::i2c::COMMAND,
                                self.i2c_csr.ms(utra::i2c::COMMAND_RD, 1) |
                                self.i2c_csr.ms(utra::i2c::COMMAND_STO, 1) |
                                self.i2c_csr.ms(utra::i2c::COMMAND_ACK, 1)
                            );
                        } else {
                            self.i2c_csr.wfo(utra::i2c::COMMAND_RD, 1);
                        }
                        self.index += 1;
                    } else {
                        self.report_read_done();
                        self.state = I2cState::Idle;
                    }
                } else {
                    // we should never get here, because rxbuf was checked as Some() by the setup routine
                    log::error!("LLIO|I2C: illegal read state");
                }
            },
            I2cState::Idle => {
                log::error!("LLIO|I2C: received interrupt event when no transaciton pending!");
            }
        }
    }
}
