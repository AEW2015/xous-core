use crate::{ShellCmdApi, CommonEnv};
use xous_ipc::String;

#[derive(Debug)]
pub struct JtagCmd {
    jtag: jtag::Jtag,
}
impl JtagCmd {
    pub fn new(xns: &xous_names::XousNames) -> JtagCmd {
        JtagCmd {
            jtag: jtag::Jtag::new(&xns).expect("couldn't connect to JTAG block"),
        }
    }
}

impl<'a> ShellCmdApi<'a> for JtagCmd {
    cmd_api!(jtag); // inserts boilerplate for command API

    fn process(&mut self, args: String::<1024>, _env: &mut CommonEnv) -> Result<Option<String::<1024>>, xous::Error> {
        use core::fmt::Write;
        let mut ret = String::<1024>::new();
        let helpstring = "jtag [id] [dna] [efuse] [reset] [burn0] [wbstar]";

        let mut tokens = args.as_str().unwrap().split(' ');

        if let Some(sub_cmd) = tokens.next() {
            match sub_cmd {
                "id" => {
                    let id = self.jtag.get_id().unwrap();
                    write!(ret, "JTAG idcode: 0x{:x}", id).unwrap();
                }
                "dna" => {
                    let dna= self.jtag.get_dna().unwrap();
                    write!(ret, "JTAG idcode: 0x{:x}", dna).unwrap();
                }
                "efuse" => {
                    let efuse = self.jtag.efuse_fetch().unwrap();
                    write!(ret, "User: 0x{:x}\nCntl: 0x{:x}\n,Fuse: {:x?}", efuse.user, efuse.cntl, efuse.key).unwrap();
                }
                "ir" => {
                    if let Some(val) = tokens.next() {
                        let intval = u8::from_str_radix(val, 2).unwrap();
                        self.jtag.write_ir(intval).unwrap();
                        write!(ret, "sending IR of 0x{:x}", intval).unwrap();
                    } else {
                        write!(ret, "ir needs an argument!").unwrap();
                    }
                }
                "burn0" => {
                    match self.jtag.efuse_key_burn([0; 32]) {
                        Ok(res) => {
                            if res {
                                write!(ret, "efuse key dummy burn was successful").unwrap();
                            } else {
                                write!(ret, "efuse key dummy burn was a failure").unwrap();
                            }
                        }
                        Err(e) => {
                            write!(ret, "internal error in doing efuse dummy key burn: {:?}", e).unwrap();
                        }
                    }
                }
                "wbstar" => {
                    write!(ret,"Hello World! ").unwrap();
                    if let Some(sub_sub_cmd) = tokens.next() {
                        match sub_sub_cmd {
                            "get" => {
                                write!(ret, "What about get?!").unwrap();
                            }                            
                            "set" => {
                                if let Some(set_value) = tokens.next() {
                                    let without_prefix = set_value.trim_start_matches("0x");
                                    let intval = u32::from_str_radix(without_prefix, 16).unwrap();
                                    write!(ret, "Can't set wbstar to 0x{:x} yet!", intval).unwrap();
                                    self.jtag.write_wbstar(intval).unwrap();
                                    write!(ret, "Did it!").unwrap();
                                }
                                else {
                                    write!(ret, "jtag wbstar set [<addr>]").unwrap();
                                }
                            }
                            _ => {
                                write!(ret, "{} not implmented yet!", sub_sub_cmd).unwrap();
                            }
                        }
                        

                    } else {
                        write!(ret, "jtag wbstar [get] [set <addr>]").unwrap();
                    }                    
                }
                _ => {
                    write!(ret, "{}", helpstring).unwrap();
                }
            }

        } else {
            write!(ret, "{}", helpstring).unwrap();
        }
        Ok(Some(ret))
    }
}
