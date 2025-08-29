//! Models selected NetEm configuration capabilities.
//! For further information on the configuration options, see [Config].

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::error::Error::{self, LossNegative, LossTooHigh};

/// The NetEm configuration options supported.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// The delay with which a packet should be transmitted.
    delay: u16,
    /// The delay jitter with which a packet should be transmitted.
    jitter: u16,
    /// The loss percentage with which a packet should be transmitted.
    loss: Loss,
}

impl Config {
    /// Create a new [Config].
    ///
    /// # Arguments
    ///
    /// * `delay` - The delay ith which a packet should be transmitted.
    /// * `jitter` - The delay jitter with which a packet should be transmitted.
    /// * `loss` - The loss percentage with which a packet should be transmitted.
    /// * `distribution` - The loss distribution with which a packet should be transmitted.
    pub fn new(delay: u16, jitter: u16, loss: Loss) -> Self {
        Self {
            delay,
            jitter,
            loss,
        }
    }

    pub(crate) fn to_tc_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        let mut delay = self.delay.to_string();
        delay.push_str("ms");
        args.push("delay".to_string());
        args.push(delay);

        let mut jitter = self.jitter.to_string();
        jitter.push_str("ms");
        args.push(jitter);

        let mut loss = self.loss.to_string();
        loss.push('%');
        args.push("loss".to_string());
        args.push(loss);

        if self.delay != 0 && self.jitter != 0 {
            args.push("distribution".to_string());
            args.push(Distribution::Normal.to_string());
        }

        args
    }
}

/// Models the loss parameter of a NetEm configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Loss(f64);

impl Loss {
    /// Creates a new [Loss] from a [f64].
    /// An error is returned if the provided loss
    /// is not non-negative or if it is not less than 100.
    ///
    /// # Arguments
    ///
    /// * `loss` - The loss percentage with which a packet should be transmitted.
    pub fn new(loss: f64) -> Result<Self, Error> {
        if loss.signum() == -1.0 {
            return Err(LossNegative { loss });
        }
        if loss > 100.0 {
            return Err(LossTooHigh { loss });
        }
        Ok(Self(loss))
    }
}

impl fmt::Display for Loss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Enumerates the different loss distribution options.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Distribution {
    /// The normal loss distribution.
    Normal,
}

impl fmt::Display for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Distribution::Normal => write!(f, "normal"),
        }
    }
}
