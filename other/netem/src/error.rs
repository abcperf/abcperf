//! Provides the errors that may possibly occur when using TCNetemTracker.

use std::{io::Error as IOError, net::IpAddr, process::ExitStatus};

use crate::{Device, DstAddr, Handle};

/// The error types that may possibly occur when using [crate::TCNetemTracker].
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Packet loss must be a non-negative percentage value.
    #[error("The provided packet loss ({loss:?}) must be non-negative.")]
    LossNegative { loss: f64 },
    /// Packet loss must be lower than 100.
    #[error("The provided packet loss ({loss:?}) must be lower than 100.")]
    LossTooHigh { loss: f64 },
    /// Failed to remove NetEm configuration of device for outgoing traffic to given destination address because no config exists.
    #[error("Failed to update egress NetEm configuration for device ({dev:?}), destination: {dst:?}): No config for the device and destination exists.")]
    RemoveFailureConfigDoesNotExist { dev: String, dst: IpAddr },
    /// Failed to create bands for new NetEm configuration.
    #[error("Failed to create bands for new NetEm configuration: {io_error:?}.")]
    BandsCreationFailure { io_error: IOError },
    /// Failed to configure band with NetEm configuration.
    #[error("Failed to configure band ({band:?}) with NetEm configuration: {io_error:?}.")]
    BandConfigFailure { band: u16, io_error: IOError },
    /// Failed to run tc command to clean all NetEm configurations.
    #[error("Failed to run tc command to clean all NetEm configuration: {io_error:?}.")]
    TCNetemCleanupFailure { io_error: IOError },
    /// Failed to redirect traffic with a particular destination to a specific band.
    #[error("Failed to redirect traffic with destination {dst:?} to band {band:?}: {io_error:?}")]
    TrafficRedirectionFailure {
        dst: IpAddr,
        band: u16,
        io_error: IOError,
    },
    /// Did not exit tc command successfully.
    #[error("Failed to execute tc command: Did not exit successfully: ({status:?}).")]
    UnsuccessfulTCExit { status: ExitStatus },
    /// The execution of `ip route get` with the provided address failed.
    #[error("The execution of ip route get {dst:?} failed: {exit_status:?}.")]
    IpRouteGetFailed {
        dst: IpAddr,
        exit_status: ExitStatus,
    },
    /// The execution of `ip route get` with the provided address failed.
    #[error("The execution of ip route get {dst:?} failed: {err:?}.")]
    IpRouteGetFailedIO { dst: IpAddr, err: IOError },
    /// Could not retrieve information on the network device.
    #[error("Could not retrieve information on the network device.")]
    NetDeviceFailure,
    /// Failed to configure the root handle of the device.
    #[error("Failed to configure the root handle of device {dev:?}: {err:?}")]
    ConfigRootHandle { dev: Device, err: IOError },
    /// Failed to add the desired class to the device.
    #[error("Failed to add the desired class {classid:?} to device {dev:?}: {err:?}")]
    AddClass {
        dev: Device,
        classid: String,
        err: IOError,
    },
    /// Failed to delete the desired class of the device.
    #[error("Failed to delete the desired class {classid:?} of device {dev:?}: {err:?}")]
    DelClass {
        dev: Device,
        classid: String,
        err: IOError,
    },
    /// Failed to set qdisc netem handle for the device.
    #[error("Failed to set qdisc netem handle {handle:?} for device {dev:?}: {err:?}")]
    SetQdiscNetemHandle {
        dev: Device,
        handle: Handle,
        err: IOError,
    },
    /// Failed to delete qdisc netem handle of the device.
    #[error("Failed to delete qdisc netem handle {handle:?} of device {dev:?}: {err:?}")]
    DelQdiscNetemHandle {
        dev: Device,
        handle: Handle,
        err: IOError,
    },
    /// Failed to add the desired filter to the netem handle of the device for the provided destination.
    #[error("Failed to add the desired filter to the netem handle {handle:?} of device {dev:?} for the provided destination {dst:?}: {err:?}")]
    AddFilterNetemHandle {
        dev: Device,
        handle: Handle,
        dst: DstAddr,
        err: IOError,
    },
    /// Failed to delete the desired filter to the netem handle of the device for the provided destination.
    #[error("Failed to delete the desired filter to the netem handle {handle:?} of device {dev:?} for the provided destination {dst:?}: {err:?}")]
    DelFilterNetemHandle {
        dev: Device,
        handle: Handle,
        dst: DstAddr,
        err: IOError,
    },
}
