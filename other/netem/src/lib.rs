//! Supports the emulation of outgoing network traffic depending on its destination.

pub mod config;
mod error;

use std::{borrow::Cow, collections::HashMap, fmt, net::IpAddr, process::Command};

use anyhow::Result;
use config::Config;
use error::Error::{
    self, AddClass, AddFilterNetemHandle, ConfigRootHandle, DelClass, DelQdiscNetemHandle,
    SetQdiscNetemHandle, TCNetemCleanupFailure, UnsuccessfulTCExit,
};
use tracing::debug;

type DstAddr = IpAddr;
type Device = String;
type Handle = u64;
type CounterHandles = u64;

const ROOT_HANDLE: &str = "1:";
const HTB_RATE: &str = "10Gbit";

/// Provides a way of adding, changing, and removing NetEm configurations
/// while taking the destination of the packets into account.
#[derive(Debug, Clone)]
pub struct TCNetemTracker {
    /// The classid of the root handle.
    root_classid: String,
    /// Keeps track of the handles created for each device and destination.
    dev_dst_handles: HashMap<(Device, DstAddr), Handle>,
    /// Keeps track of the amount of handles created for a device.
    dev_counter_handles: HashMap<Device, CounterHandles>,
}

impl Default for TCNetemTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl TCNetemTracker {
    /// Creates a new [TCNetemTracker] with default values.
    /// Initializes the classid of the root handle.
    fn new() -> Self {
        let mut root_classid = ROOT_HANDLE.to_string();
        root_classid.push('1');
        Self {
            root_classid,
            dev_dst_handles: HashMap::new(),
            dev_counter_handles: HashMap::new(),
        }
    }
    /// Adds a new or updates an existing NetEm egress configuration for a
    /// given destination address to the source's device.
    ///
    /// The source's device is selected depending on the device that is used
    /// for reaching the destination address.
    ///
    /// The existing configuration is updated if there is already an existing
    /// NetEm egress configuration for the given destination address, otherwise
    /// the provided configuration is added as a new config.
    ///
    ///
    /// # Arguments
    ///
    /// * `dst` - The destination address for which the traffic should be configured with NetEm.
    /// * `dev` - The source's device which should be configured.
    /// * `netem_config` - The NetEm configuration which should be applied to outgoing traffic with the provided destination.
    pub fn config_egress(&mut self, dst: IpAddr, netem_config: &Config) -> Result<(), Error> {
        // Obtain device name.
        let dev = get_device(dst).unwrap();
        // Call correct fn depending on whether there is already a NetEm
        // configuration for the given device of the source or not.
        debug!("Configuring device {dev:?}");

        match self.dev_dst_handles.get(&(dev.clone(), dst)) {
            Some(netem_handle_to_update) => self.set_qdisc_netem_handle(
                &QdiscOption::Change,
                &dev,
                *netem_handle_to_update,
                netem_config,
            ),
            None => self.add_new_config_egress(dev, dst, netem_config),
        }
    }

    /// Configures the root handle of the device.
    ///
    /// # Arguments
    ///
    /// * `dev` - The device for which a root handle should be configured.
    /// * `qdisc_option` - The option of how the root handle should be
    ///                    configured.
    fn config_dev_root_handle(
        &mut self,
        dev: Device,
        qdisc_option: &QdiscOption,
    ) -> Result<(), Error> {
        let mut program = Command::new("tc");
        let command = program.args([
            "qdisc",
            &qdisc_option.to_string(),
            "dev",
            &dev,
            "root",
            "handle",
            ROOT_HANDLE,
            "htb",
        ]);

        debug!("Configuring root handle with command {command:?}");
        match command.status() {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => return Err(ConfigRootHandle { dev, err: io_error }),
        }

        self.add_dev_class(dev).map(|_| ())
    }

    /// Adds a class for the given device.
    /// The class is later used to create a netem handle.
    fn add_dev_class(&mut self, dev: Device) -> Result<Handle, Error> {
        let counter_handles = self.dev_counter_handles.entry(dev.clone()).or_insert(0);
        *counter_handles += 1;
        let (parent, classid) = if *counter_handles == 1 {
            (ROOT_HANDLE.to_string(), self.root_classid.clone())
        } else {
            let next_classid = classid_from_handle(*counter_handles);
            (self.root_classid.clone(), next_classid)
        };

        let mut program = Command::new("tc");
        let command = program.args([
            "class",
            &ClassOption::Add.to_string(),
            "dev",
            &dev,
            "parent",
            &parent,
            "classid",
            &classid,
            "htb",
            "rate",
            HTB_RATE,
        ]);

        debug!("Adding class with command {command:?}");

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => {
                return Err(AddClass {
                    dev,
                    classid,
                    err: io_error,
                })
            }
        }

        Ok(*counter_handles)
    }

    fn del_dev_class(&mut self, dev: Device, dst: DstAddr) -> Result<(), Error> {
        let parent = &self.root_classid;
        let classid = match self.dev_dst_handles.get(&(dev.clone(), dst)) {
            Some(handle) => classid_from_handle(*handle),
            None => todo!(),
        };

        match Command::new("tc")
            .args([
                "class",
                &ClassOption::Delete.to_string(),
                "dev",
                &dev,
                "parent",
                parent,
                "classid",
                &classid,
                "htb",
                "rate",
                HTB_RATE,
            ])
            .status()
        {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => {
                return Err(DelClass {
                    dev,
                    classid,
                    err: io_error,
                })
            }
        }

        Ok(())
    }

    fn set_qdisc_netem_handle(
        &mut self,
        qdisc_option: &QdiscOption,
        dev: &Device,
        netem_handle: Handle,
        netem_config: &Config,
    ) -> Result<(), Error> {
        let mut handle = netem_handle.to_string();
        handle.push(':');
        let mut parent = ROOT_HANDLE.to_string();
        parent.push_str(&netem_handle.to_string());

        let mut program = Command::new("tc");
        let command = program
            .args([
                "qdisc",
                &qdisc_option.to_string(),
                "dev",
                dev,
                "handle",
                &handle,
                "parent",
                &parent,
            ])
            .arg("netem")
            .args(netem_config.to_tc_args());

        debug!("Setting netem handle with command {command:?}");

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => {
                return Err(SetQdiscNetemHandle {
                    dev: dev.to_string(),
                    handle: netem_handle,
                    err: io_error,
                })
            }
        }
        Ok(())
    }

    fn del_qdisc_netem_handle(&mut self, dev: &Device, dst: DstAddr) -> Result<(), Error> {
        let handle = match self.dev_dst_handles.get(&(dev.clone(), dst)) {
            Some(handle) => handle,
            None => todo!(),
        };

        let argv = create_qdisc_netem_command(*handle, &QdiscOption::Delete, dev.to_string(), None);
        let mut program = Command::new(argv[0].clone());
        let command = program.args(&argv[..]);

        debug!("Deleting netem handle with command {command:?}");

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => {
                return Err(DelQdiscNetemHandle {
                    dev: dev.to_string(),
                    handle: *handle,
                    err: io_error,
                })
            }
        }
        Ok(())
    }

    fn add_filter_netem_handle(
        &mut self,
        dev: Device,
        handle: Handle,
        dst: DstAddr,
    ) -> Result<(), Error> {
        let argv = create_filter_command(&FilterOption::Add, &dev, handle, dst);
        let argv = &argv[..];

        let mut program = Command::new(&argv[0]);
        let command = program.args(&argv[1..]);

        debug!("Adding filter with command {command:?}");

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    return Err(UnsuccessfulTCExit { status });
                }
            }
            Err(io_error) => {
                return Err(AddFilterNetemHandle {
                    dev,
                    handle,
                    dst,
                    err: io_error,
                })
            }
        };
        Ok(())
    }

    fn remove_filter_netem_handle(&mut self, dev: &Device, dst: DstAddr) -> Result<(), Error> {
        let handle = self.dev_dst_handles.get(&(dev.clone(), dst));
        match handle {
            Some(handle) => {
                let argv = create_filter_command(&FilterOption::Delete, dev, *handle, dst);

                let mut program = Command::new(&argv[0]);
                let command = program.args(&argv[1..]);

                debug!("Removing filter with command {command:?}");

                match command.status() {
                    Ok(status) => {
                        if !status.success() {
                            return Err(UnsuccessfulTCExit { status });
                        }
                    }
                    Err(io_error) => {
                        return Err(AddFilterNetemHandle {
                            dev: dev.to_string(),
                            handle: *handle,
                            dst,
                            err: io_error,
                        })
                    }
                }
                Ok(())
            }
            None => todo!(),
        }
    }

    /// Adds a new NetEm egress configuration for a given destination address
    /// to the source's device.
    ///
    /// The source's device is selected depending on the device that is used
    /// for reaching the destination address.
    ///
    /// An error is returned if a configuration already exists.
    /// In order to change or remove the configuration, the usage of the other respective functions is encouraged.
    ///
    /// # Arguments
    ///
    /// * `dst` - The destination address for which the traffic should be configured with NetEm.
    /// * `dev` - The source's device which should be configured.
    /// * `netem_config` - The NetEm configuration which should be applied to outgoing traffic with the provided destination.
    fn add_new_config_egress(
        &mut self,
        dev: Device,
        dst: IpAddr,
        netem_config: &Config,
    ) -> Result<(), Error> {
        match self.dev_counter_handles.get(&dev) {
            Some(counter_handles) if *counter_handles != 0 => {}
            _ => {
                self.config_dev_root_handle(dev.clone(), &QdiscOption::Add)?;
            }
        }

        let netem_handle_to_config = self.add_dev_class(dev.clone())?;
        self.set_qdisc_netem_handle(
            &QdiscOption::Add,
            &dev,
            netem_handle_to_config,
            netem_config,
        )?;
        self.add_filter_netem_handle(dev.clone(), netem_handle_to_config, dst)?;

        self.dev_dst_handles
            .insert((dev, dst), netem_handle_to_config);

        Ok(())
    }

    /// Removes a previously added and not removed NetEm egress configuration
    /// of the specified source's device for a given destination.
    ///
    /// An error is returned if a given source's device
    /// does not contain a separate configuration for the given destination.
    ///
    /// # Arguments
    ///
    /// * `dev` - The source's device which contains a (to-be-deleted) NetEm configuration for outgoing traffic with the provided destination.
    /// * `dst` - The destination address for which the existing NetEm configuration should be deleted.
    pub fn remove_config_egress(&mut self, dst: IpAddr) -> Result<(), Error> {
        // Obtain device name.
        let dev = get_device(dst).unwrap();

        self.remove_filter_netem_handle(&dev, dst)?;
        self.del_qdisc_netem_handle(&dev, dst)?;
        self.del_dev_class(dev.clone(), dst)?;

        self.dev_dst_handles.remove(&(dev, dst));

        Ok(())
    }

    /// Cleans all NetEm configurations created with this instance.
    /// Returns an error if the tc command to clean all NetEm configurations
    /// could not be successfully run or exited.
    pub fn clean_all_netem_configs(&mut self) -> Result<(), Error> {
        for dev in self.dev_counter_handles.keys() {
            let mut main_command = Command::new("tc");
            let command = main_command.args([
                "qdisc",
                &QdiscOption::Delete.to_string(),
                "dev",
                dev,
                "root",
                "handle",
                ROOT_HANDLE,
                "htb",
            ]);

            debug!("Running command to clean all configs {command:?}");

            match command.status() {
                Ok(status) => {
                    if !status.success() {
                        return Err(UnsuccessfulTCExit { status });
                    }
                }
                Err(io_error) => return Err(TCNetemCleanupFailure { io_error }),
            }
        }
        self.dev_dst_handles.clear();
        self.dev_counter_handles.clear();
        Ok(())
    }
}

fn classid_from_handle(handle: Handle) -> String {
    let mut classid = ROOT_HANDLE.to_string();
    classid.push_str(&(handle).to_string());
    classid
}

fn create_qdisc_netem_command(
    handle: Handle,
    qdisc_option: &QdiscOption,
    dev: Device,
    netem_config: Option<&Config>,
) -> Vec<String> {
    let mut handle = handle.to_string();
    handle.push(':');
    let mut parent = ROOT_HANDLE.to_string();
    parent.push_str(&handle.to_string());

    let mut argv = vec![
        "tc".to_string(),
        "qdisc".to_string(),
        qdisc_option.to_string(),
        "dev".to_string(),
        dev,
        "handle".to_string(),
        handle,
        "parent".to_string(),
        parent,
    ];

    match qdisc_option {
        QdiscOption::Delete => argv,
        _ => match netem_config {
            Some(netem_config) => {
                argv.push("netem".to_string());
                argv.append(&mut netem_config.to_tc_args());
                argv
            }
            None => todo!(),
        },
    }
}

fn create_filter_command(
    filter_option: &FilterOption,
    dev: &Device,
    handle: Handle,
    dst: DstAddr,
) -> Vec<String> {
    let protocol = match dst {
        IpAddr::V4(_) => ("ip", "ip"),
        IpAddr::V6(_) => ("ipv6", "ip6"),
    };
    let mut flow_id = ROOT_HANDLE.to_string();
    flow_id.push_str(&handle.to_string());

    vec![
        "tc".to_string(),
        "filter".to_string(),
        filter_option.to_string(),
        "dev".to_string(),
        (&dev).to_string(),
        "pref".to_string(),
        handle.to_string(),
        "protocol".to_string(),
        protocol.0.to_string(),
        "u32".to_string(),
        "match".to_string(),
        protocol.1.to_string(),
        "dst".to_string(),
        dst.to_string(),
        "flowid".to_string(),
        flow_id.to_string(),
    ]
}

/// Enumerates the different supported qdisc options.
enum QdiscOption {
    /// The option to add a qdisc.
    Add,
    /// The option to change a qdisc.
    Change,
    /// The option to delete a qdisc.
    Delete,
}

impl fmt::Display for QdiscOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QdiscOption::Add => write!(f, "add"),
            QdiscOption::Change => write!(f, "change"),
            QdiscOption::Delete => write!(f, "del"),
        }
    }
}

/// Enumerates the different supported class options.
enum ClassOption {
    /// The option to add a class.
    Add,
    /// The option to delete a class.
    Delete,
}

impl fmt::Display for ClassOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClassOption::Add => write!(f, "add"),
            ClassOption::Delete => write!(f, "change"),
        }
    }
}

/// Enumerates the different supported filter options.
enum FilterOption {
    /// The filter option add.
    Add,
    /// The filter option delete.
    Delete,
}

impl fmt::Display for FilterOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterOption::Add => write!(f, "add"),
            FilterOption::Delete => write!(f, "del"),
        }
    }
}

/// Obtain the network device depending on the destination address.
///
/// # Arguments
///
/// * `dst` - The destination address to be used.
fn get_device(dst: IpAddr) -> Result<String> {
    match Command::new("ip")
        .args(["route", "get", &dst.to_string()])
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Err(Error::IpRouteGetFailed {
                    dst,
                    exit_status: output.status,
                }
                .into());
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_device_from_stdout(stdout)
        }
        Err(io_error) => Err(Error::IpRouteGetFailedIO { dst, err: io_error }.into()),
    }
}

/// Parses the device name from ip route's stdout.
///
/// # Arguments
///
/// * `stdout` - The stdout of ip route's call.
fn parse_device_from_stdout(stdout: Cow<'_, str>) -> Result<String> {
    let mut stdout_by_space = stdout.split_whitespace();

    for stdout_word in stdout_by_space.by_ref() {
        if stdout_word.eq("dev") {
            break;
        }
    }
    match stdout_by_space.next() {
        Some(dev) => Ok(dev.to_owned()),
        None => Err(Error::NetDeviceFailure.into()),
    }
}
