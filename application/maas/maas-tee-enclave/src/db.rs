use std::collections::{HashMap, HashSet};

use maas_types::{CheckinMsg, CheckoutMsg, Interaction, Pseu, RegisterMsg, Secret};
use serde::{Deserialize, Serialize};
use sgx_types::types::Key128bit;
use std::vec::Vec;

#[derive(Serialize, Deserialize)]
pub(super) struct ModifyMsg {
    pub(super) inner: InnerModifyMsg,
    pub(super) enc_key: Key128bit,
}

#[derive(Serialize, Deserialize)]
pub(super) enum InnerModifyMsg {
    Register(RegisterMsg),
    Checkin(CheckinMsg),
    Checkout(CheckoutMsg),
    Reset(ResetMsg),
    Reregister(ReregisterMsg),
    Clearance,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ResetMsg {
    long_term_secret: Secret,
    new_short_term_secret: Secret,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ReregisterMsg {
    new_long_term_secret: Secret,
    new_short_term_secret: Secret,
}

struct Trip {
    trip_id: u64,
    checkin_location: (),
    checkin_time: (),
    checkout_location: (),
    checkout_time: (),
}

struct RunningTrip {
    trip_id: u64,
    checkin_location: (),
    checkin_time: (),
}

struct UserData {
    pseu: Pseu,
    next_short_term_secret: Secret,
    current_trip: Option<RunningTrip>,
    trips: Vec<Trip>,
}

#[derive(Default)]
pub(super) struct Db {
    next_trip_id: u64,
    users: HashMap<Secret, UserData>,
    used_pseus: HashSet<Pseu>,
}

impl Db {
    pub(super) fn modify(&mut self, msg: ModifyMsg) -> Result<(), ()> {
        match msg.inner {
            InnerModifyMsg::Register(reg) => {
                // TODO check sig

                if self.used_pseus.contains(&reg.cack.pseu) {
                    return Err(());
                }
                if self.users.contains_key(&reg.long_term_secret) {
                    return Err(());
                }
                self.used_pseus.insert(reg.cack.pseu);
                self.users.insert(
                    reg.long_term_secret,
                    UserData {
                        pseu: reg.cack.pseu,
                        next_short_term_secret: reg.short_term_secret,
                        current_trip: None,
                        trips: Vec::new(),
                    },
                );
                Ok(())
            }
            InnerModifyMsg::Checkin(CheckinMsg {
                interaction:
                    Interaction {
                        short_term_secret,
                        timestamp,
                        location,
                        sig,
                    },
                long_term_secret,
                next_short_term_secret,
            }) => {
                self.users
                    .entry(long_term_secret)
                    .or_insert_with(|| UserData {
                        pseu: 0,
                        next_short_term_secret: short_term_secret,
                        current_trip: None,
                        trips: Vec::new(),
                    });
                if let Some(user) = self.users.get_mut(&long_term_secret) {
                    if user.next_short_term_secret == short_term_secret {
                        if user.current_trip.is_some() {
                            return Err(());
                        }
                        let trip_id = self.next_trip_id;
                        self.next_trip_id += 1;
                        // TODO check sig
                        // TODO check time
                        user.next_short_term_secret = next_short_term_secret;
                        user.current_trip = Some(RunningTrip {
                            trip_id,
                            checkin_time: timestamp,
                            checkin_location: location,
                        });
                        Ok(())
                    } else {
                        Err(())
                    }
                } else {
                    Err(())
                }
            }
            InnerModifyMsg::Checkout(CheckoutMsg {
                interaction:
                    Interaction {
                        short_term_secret,
                        timestamp,
                        location,
                        sig,
                    },
                long_term_secret,
                next_short_term_secret,
            }) => {
                self.users
                    .entry(long_term_secret)
                    .or_insert_with(|| UserData {
                        pseu: 0,
                        next_short_term_secret: short_term_secret,
                        current_trip: None,
                        trips: Vec::new(),
                    });
                if let Some(user) = self.users.get_mut(&long_term_secret) {
                    if user.next_short_term_secret == short_term_secret {
                        // TODO check sig
                        // TODO check time
                        if let Some(RunningTrip {
                            trip_id,
                            checkin_location,
                            checkin_time,
                        }) = user.current_trip.take()
                        {
                            user.next_short_term_secret = next_short_term_secret;
                            user.trips.push(Trip {
                                trip_id,
                                checkin_location,
                                checkin_time,
                                checkout_location: location,
                                checkout_time: timestamp,
                            });
                            Ok(())
                        } else {
                            Err(())
                        }
                    } else {
                        Err(())
                    }
                } else {
                    Err(())
                }
            }
            InnerModifyMsg::Reset(reset) => {
                if let Some(user) = self.users.get_mut(&reset.long_term_secret) {
                    user.next_short_term_secret = reset.new_short_term_secret;
                    Ok(())
                } else {
                    unreachable!()
                }
            }
            InnerModifyMsg::Reregister(_) => {
                // TODO figure out if we need more info, seems linkely
                Err(())
            }
            InnerModifyMsg::Clearance => todo!(), //ModifyMsg::Clearance => todo!(),
        }
    }
}
