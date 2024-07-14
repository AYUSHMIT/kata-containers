// Copyright (c) 2019 Ant Financial
//
// SPDX-License-Identifier: Apache-2.0
//
use crate::rpc;
use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;
use std::str::FromStr;
use std::time;
use strum_macros::{Display, EnumString};
use tracing::instrument;
use url::Url;

use kata_types::config::default::DEFAULT_AGENT_VSOCK_PORT;

const DEBUG_CONSOLE_FLAG: &str = "agent.debug_console";
const DEV_MODE_FLAG: &str = "agent.devmode";
const TRACE_MODE_OPTION: &str = "agent.trace";
const LOG_LEVEL_OPTION: &str = "agent.log";
const SERVER_ADDR_OPTION: &str = "agent.server_addr";
const PASSFD_LISTENER_PORT: &str = "agent.passfd_listener_port";
const HOTPLUG_TIMOUT_OPTION: &str = "agent.hotplug_timeout";
const CDH_API_TIMOUT_OPTION: &str = "agent.cdh_api_timeout";
const DEBUG_CONSOLE_VPORT_OPTION: &str = "agent.debug_console_vport";
const LOG_VPORT_OPTION: &str = "agent.log_vport";
const CONTAINER_PIPE_SIZE_OPTION: &str = "agent.container_pipe_size";
const UNIFIED_CGROUP_HIERARCHY_OPTION: &str = "systemd.unified_cgroup_hierarchy";
const CONFIG_FILE: &str = "agent.config_file";
const GUEST_COMPONENTS_REST_API_OPTION: &str = "agent.guest_components_rest_api";
const GUEST_COMPONENTS_PROCS_OPTION: &str = "agent.guest_components_procs";
#[cfg(feature = "guest-pull")]
const IMAGE_REGISTRY_AUTH_OPTION: &str = "agent.image_registry_auth";
const SECURE_STORAGE_INTEGRITY_OPTION: &str = "agent.secure_storage_integrity";

#[cfg(feature = "guest-pull")]
const ENABLE_SIGNATURE_VERIFICATION: &str = "agent.enable_signature_verification";

#[cfg(feature = "guest-pull")]
const IMAGE_POLICY_FILE: &str = "agent.image_policy_file";

// Configure the proxy settings for HTTPS requests in the guest,
// to solve the problem of not being able to access the specified image in some cases.
const HTTPS_PROXY: &str = "agent.https_proxy";
const NO_PROXY: &str = "agent.no_proxy";

const DEFAULT_LOG_LEVEL: slog::Level = slog::Level::Info;
const DEFAULT_HOTPLUG_TIMEOUT: time::Duration = time::Duration::from_secs(3);
const DEFAULT_CDH_API_TIMEOUT: time::Duration = time::Duration::from_secs(50);
const DEFAULT_CONTAINER_PIPE_SIZE: i32 = 0;
const VSOCK_ADDR: &str = "vsock://-1";

// Environment variables used for development and testing
const SERVER_ADDR_ENV_VAR: &str = "KATA_AGENT_SERVER_ADDR";
const LOG_LEVEL_ENV_VAR: &str = "KATA_AGENT_LOG_LEVEL";
const TRACING_ENV_VAR: &str = "KATA_AGENT_TRACING";

const ERR_INVALID_LOG_LEVEL: &str = "invalid log level";
const ERR_INVALID_LOG_LEVEL_PARAM: &str = "invalid log level parameter";
const ERR_INVALID_GET_VALUE_PARAM: &str = "expected name=value";
const ERR_INVALID_GET_VALUE_NO_NAME: &str = "name=value parameter missing name";
const ERR_INVALID_GET_VALUE_NO_VALUE: &str = "name=value parameter missing value";
const ERR_INVALID_LOG_LEVEL_KEY: &str = "invalid log level key name";
const ERR_INVALID_TIMEOUT: &str = "invalid timeout parameter";
const ERR_INVALID_TIMEOUT_PARAM: &str = "unable to parse timeout";
const ERR_INVALID_TIMEOUT_KEY: &str = "invalid timeout key name";

const ERR_INVALID_CONTAINER_PIPE_SIZE: &str = "invalid container pipe size parameter";
const ERR_INVALID_CONTAINER_PIPE_SIZE_PARAM: &str = "unable to parse container pipe size";
const ERR_INVALID_CONTAINER_PIPE_SIZE_KEY: &str = "invalid container pipe size key name";
const ERR_INVALID_CONTAINER_PIPE_NEGATIVE: &str = "container pipe size should not be negative";

const ERR_INVALID_GUEST_COMPONENTS_REST_API_VALUE: &str = "invalid guest components rest api feature given. Valid values are `all`, `attestation`, `resource`";
const ERR_INVALID_GUEST_COMPONENTS_PROCS_VALUE: &str = "invalid guest components process param given. Valid values are `attestation-agent`, `confidential-data-hub`, `api-server-rest`, or `none`";

#[derive(Clone, Copy, Debug, Default, Display, Deserialize, EnumString, PartialEq)]
// Features seem to typically be in kebab-case format, but we only have single words at the moment
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum GuestComponentsFeatures {
    All,
    Attestation,
    #[default]
    Resource,
}

#[derive(Clone, Copy, Debug, Default, Display, Deserialize, EnumString, PartialEq)]
/// Attestation-related processes that we want to spawn as children of the agent
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum GuestComponentsProcs {
    None,
    /// ApiServerRest implies ConfidentialDataHub and AttestationAgent
    #[default]
    ApiServerRest,
    AttestationAgent,
    /// ConfidentialDataHub implies AttestationAgent
    ConfidentialDataHub,
}

#[derive(Debug)]
pub struct AgentConfig {
    pub debug_console: bool,
    pub dev_mode: bool,
    pub log_level: slog::Level,
    pub hotplug_timeout: time::Duration,
    pub cdh_api_timeout: time::Duration,
    pub debug_console_vport: i32,
    pub log_vport: i32,
    pub container_pipe_size: i32,
    pub server_addr: String,
    pub passfd_listener_port: i32,
    pub unified_cgroup_hierarchy: bool,
    pub tracing: bool,
    pub supports_seccomp: bool,
    pub https_proxy: String,
    pub no_proxy: String,
    pub guest_components_rest_api: GuestComponentsFeatures,
    pub guest_components_procs: GuestComponentsProcs,
    #[cfg(feature = "guest-pull")]
    pub image_registry_auth: String,
    pub secure_storage_integrity: bool,
    #[cfg(feature = "guest-pull")]
    pub enable_signature_verification: bool,
    #[cfg(feature = "guest-pull")]
    pub image_policy_file: String,
}

#[derive(Debug, Deserialize)]
pub struct AgentConfigBuilder {
    pub debug_console: Option<bool>,
    pub dev_mode: Option<bool>,
    pub log_level: Option<String>,
    pub hotplug_timeout: Option<time::Duration>,
    pub cdh_api_timeout: Option<time::Duration>,
    pub debug_console_vport: Option<i32>,
    pub log_vport: Option<i32>,
    pub container_pipe_size: Option<i32>,
    pub server_addr: Option<String>,
    pub passfd_listener_port: Option<i32>,
    pub unified_cgroup_hierarchy: Option<bool>,
    pub tracing: Option<bool>,
    pub https_proxy: Option<String>,
    pub no_proxy: Option<String>,
    pub guest_components_rest_api: Option<GuestComponentsFeatures>,
    pub guest_components_procs: Option<GuestComponentsProcs>,
    #[cfg(feature = "guest-pull")]
    pub image_registry_auth: Option<String>,
    pub secure_storage_integrity: Option<bool>,
    #[cfg(feature = "guest-pull")]
    pub enable_signature_verification: Option<bool>,
    #[cfg(feature = "guest-pull")]
    pub image_policy_file: Option<String>,
}

macro_rules! config_override {
    ($builder:ident, $config:ident, $field:ident) => {
        if let Some(v) = $builder.$field {
            $config.$field = v;
        }
    };

    ($builder:ident, $config:ident, $field:ident, $func: ident) => {
        if let Some(v) = $builder.$field {
            $config.$field = $func(&v)?;
        }
    };
}

// parse_cmdline_param parse commandline parameters.
macro_rules! parse_cmdline_param {
    // commandline flags, without func to parse the option values
    ($param:ident, $key:ident, $field:expr) => {
        if $param.eq(&$key) {
            $field = true;
            continue;
        }
    };
    // commandline options, with func to parse the option values
    ($param:ident, $key:ident, $field:expr, $func:ident) => {
        if $param.starts_with(format!("{}=", $key).as_str()) {
            let val = $func($param)?;
            $field = val;
            continue;
        }
    };
    // commandline options, with func to parse the option values, and match func
    // to valid the values
    ($param:ident, $key:ident, $field:expr, $func:ident, $guard:expr) => {
        if $param.starts_with(format!("{}=", $key).as_str()) {
            let val = $func($param)?;
            if $guard(val) {
                $field = val;
            }
            continue;
        }
    };
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            debug_console: false,
            dev_mode: false,
            log_level: DEFAULT_LOG_LEVEL,
            hotplug_timeout: DEFAULT_HOTPLUG_TIMEOUT,
            cdh_api_timeout: DEFAULT_CDH_API_TIMEOUT,
            debug_console_vport: 0,
            log_vport: 0,
            container_pipe_size: DEFAULT_CONTAINER_PIPE_SIZE,
            server_addr: format!("{}:{}", VSOCK_ADDR, DEFAULT_AGENT_VSOCK_PORT),
            passfd_listener_port: 0,
            unified_cgroup_hierarchy: false,
            tracing: false,
            supports_seccomp: rpc::have_seccomp(),
            https_proxy: String::from(""),
            no_proxy: String::from(""),
            guest_components_rest_api: GuestComponentsFeatures::default(),
            guest_components_procs: GuestComponentsProcs::default(),
            #[cfg(feature = "guest-pull")]
            image_registry_auth: String::from(""),
            secure_storage_integrity: false,
            #[cfg(feature = "guest-pull")]
            enable_signature_verification: false,
            #[cfg(feature = "guest-pull")]
            image_policy_file: String::from(""),
        }
    }
}

impl FromStr for AgentConfig {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let agent_config_builder: AgentConfigBuilder =
            toml::from_str(s).map_err(anyhow::Error::new)?;
        let mut agent_config: AgentConfig = Default::default();

        // Overwrite default values with the configuration files ones.
        config_override!(agent_config_builder, agent_config, debug_console);
        config_override!(agent_config_builder, agent_config, dev_mode);
        config_override!(
            agent_config_builder,
            agent_config,
            log_level,
            logrus_to_slog_level
        );
        config_override!(agent_config_builder, agent_config, hotplug_timeout);
        config_override!(agent_config_builder, agent_config, cdh_api_timeout);
        config_override!(agent_config_builder, agent_config, debug_console_vport);
        config_override!(agent_config_builder, agent_config, log_vport);
        config_override!(agent_config_builder, agent_config, container_pipe_size);
        config_override!(agent_config_builder, agent_config, server_addr);
        config_override!(agent_config_builder, agent_config, passfd_listener_port);
        config_override!(agent_config_builder, agent_config, unified_cgroup_hierarchy);
        config_override!(agent_config_builder, agent_config, tracing);
        config_override!(agent_config_builder, agent_config, https_proxy);
        config_override!(agent_config_builder, agent_config, no_proxy);
        config_override!(
            agent_config_builder,
            agent_config,
            guest_components_rest_api
        );
        config_override!(agent_config_builder, agent_config, guest_components_procs);
        #[cfg(feature = "guest-pull")]
        {
            config_override!(agent_config_builder, agent_config, image_registry_auth);
            config_override!(
                agent_config_builder,
                agent_config,
                enable_signature_verification
            );
            config_override!(agent_config_builder, agent_config, image_policy_file);
        }
        config_override!(agent_config_builder, agent_config, secure_storage_integrity);

        Ok(agent_config)
    }
}

impl AgentConfig {
    #[instrument]
    #[allow(clippy::redundant_closure_call)]
    pub fn from_cmdline(file: &str, args: Vec<String>) -> Result<AgentConfig> {
        // If config file specified in the args, generate our config from it
        let config_position = args.iter().position(|a| a == "--config" || a == "-c");
        if let Some(config_position) = config_position {
            if let Some(config_file) = args.get(config_position + 1) {
                let mut config =
                    AgentConfig::from_config_file(config_file).context("AgentConfig from args")?;
                config.override_config_from_envs();
                return Ok(config);
            } else {
                panic!("The config argument wasn't formed properly: {:?}", args);
            }
        }

        let mut config: AgentConfig = Default::default();
        let cmdline = fs::read_to_string(file)?;
        let params: Vec<&str> = cmdline.split_ascii_whitespace().collect();
        for param in params.iter() {
            // If we get a configuration file path from the command line, we
            // generate our config from it.
            // The agent will fail to start if the configuration file is not present,
            // or if it can't be parsed properly.
            if param.starts_with(format!("{}=", CONFIG_FILE).as_str()) {
                let config_file = get_string_value(param)?;
                return AgentConfig::from_config_file(&config_file)
                    .context("AgentConfig from kernel cmdline");
            }

            // parse cmdline flags
            parse_cmdline_param!(param, DEBUG_CONSOLE_FLAG, config.debug_console);
            parse_cmdline_param!(param, DEV_MODE_FLAG, config.dev_mode);

            // Support "bare" tracing option for backwards compatibility with
            // Kata 1.x.
            if param == &TRACE_MODE_OPTION {
                config.tracing = true;
                continue;
            }

            parse_cmdline_param!(param, TRACE_MODE_OPTION, config.tracing, get_bool_value);

            // parse cmdline options
            parse_cmdline_param!(param, LOG_LEVEL_OPTION, config.log_level, get_log_level);
            parse_cmdline_param!(
                param,
                SERVER_ADDR_OPTION,
                config.server_addr,
                get_string_value
            );

            // ensure the timeout is a positive value
            parse_cmdline_param!(
                param,
                HOTPLUG_TIMOUT_OPTION,
                config.hotplug_timeout,
                get_timeout,
                |hotplug_timeout: time::Duration| hotplug_timeout.as_secs() > 0
            );

            // ensure the timeout is a positive value
            parse_cmdline_param!(
                param,
                CDH_API_TIMOUT_OPTION,
                config.cdh_api_timeout,
                get_timeout,
                |cdh_api_timeout: time::Duration| cdh_api_timeout.as_secs() > 0
            );

            // vsock port should be positive values
            parse_cmdline_param!(
                param,
                DEBUG_CONSOLE_VPORT_OPTION,
                config.debug_console_vport,
                get_vsock_port,
                |port| port > 0
            );
            parse_cmdline_param!(
                param,
                LOG_VPORT_OPTION,
                config.log_vport,
                get_vsock_port,
                |port| port > 0
            );
            parse_cmdline_param!(
                param,
                PASSFD_LISTENER_PORT,
                config.passfd_listener_port,
                get_vsock_port,
                |port| port > 0
            );
            parse_cmdline_param!(
                param,
                CONTAINER_PIPE_SIZE_OPTION,
                config.container_pipe_size,
                get_container_pipe_size
            );
            parse_cmdline_param!(
                param,
                UNIFIED_CGROUP_HIERARCHY_OPTION,
                config.unified_cgroup_hierarchy,
                get_bool_value
            );
            parse_cmdline_param!(param, HTTPS_PROXY, config.https_proxy, get_url_value);
            parse_cmdline_param!(param, NO_PROXY, config.no_proxy, get_string_value);
            parse_cmdline_param!(
                param,
                GUEST_COMPONENTS_REST_API_OPTION,
                config.guest_components_rest_api,
                get_guest_components_features_value
            );
            parse_cmdline_param!(
                param,
                GUEST_COMPONENTS_PROCS_OPTION,
                config.guest_components_procs,
                get_guest_components_procs_value
            );
            #[cfg(feature = "guest-pull")]
            {
                parse_cmdline_param!(
                    param,
                    IMAGE_REGISTRY_AUTH_OPTION,
                    config.image_registry_auth,
                    get_string_value
                );
                parse_cmdline_param!(
                    param,
                    ENABLE_SIGNATURE_VERIFICATION,
                    config.enable_signature_verification,
                    get_bool_value
                );
                parse_cmdline_param!(
                    param,
                    IMAGE_POLICY_FILE,
                    config.image_policy_file,
                    get_string_value
                );
            }
            parse_cmdline_param!(
                param,
                SECURE_STORAGE_INTEGRITY_OPTION,
                config.secure_storage_integrity,
                get_bool_value
            );
        }

        config.override_config_from_envs();

        Ok(config)
    }

    #[instrument]
    pub fn from_config_file(file: &str) -> Result<AgentConfig> {
        let config = fs::read_to_string(file)
            .with_context(|| format!("Failed to read config file {}", file))?;
        AgentConfig::from_str(&config)
    }

    #[instrument]
    fn override_config_from_envs(&mut self) {
        if let Ok(addr) = env::var(SERVER_ADDR_ENV_VAR) {
            self.server_addr = addr;
        }

        if let Ok(addr) = env::var(LOG_LEVEL_ENV_VAR) {
            if let Ok(level) = logrus_to_slog_level(&addr) {
                self.log_level = level;
            }
        }

        if let Ok(value) = env::var(TRACING_ENV_VAR) {
            let name_value = format!("{}={}", TRACING_ENV_VAR, value);

            self.tracing = get_bool_value(&name_value).unwrap_or(false);
        }
    }
}

#[instrument]
fn get_vsock_port(p: &str) -> Result<i32> {
    let fields: Vec<&str> = p.split('=').collect();
    ensure!(fields.len() == 2, "invalid port parameter");

    Ok(fields[1].parse::<i32>()?)
}

// Map logrus (https://godoc.org/github.com/sirupsen/logrus)
// log level to the equivalent slog log levels.
//
// Note: Logrus names are used for compatability with the previous
// golang-based agent.
#[instrument]
fn logrus_to_slog_level(logrus_level: &str) -> Result<slog::Level> {
    let level = match logrus_level {
        // Note: different semantics to logrus: log, but don't panic.
        "fatal" | "panic" => slog::Level::Critical,

        "critical" => slog::Level::Critical,
        "error" => slog::Level::Error,
        "warn" | "warning" => slog::Level::Warning,
        "info" => slog::Level::Info,
        "debug" => slog::Level::Debug,

        // Not in logrus
        "trace" => slog::Level::Trace,

        _ => bail!(ERR_INVALID_LOG_LEVEL),
    };

    Ok(level)
}

#[instrument]
fn get_log_level(param: &str) -> Result<slog::Level> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() == 2, ERR_INVALID_LOG_LEVEL_PARAM);
    ensure!(fields[0] == LOG_LEVEL_OPTION, ERR_INVALID_LOG_LEVEL_KEY);

    logrus_to_slog_level(fields[1])
}

#[instrument]
fn get_timeout(param: &str) -> Result<time::Duration> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() == 2, ERR_INVALID_TIMEOUT);
    ensure!(
        matches!(fields[0], HOTPLUG_TIMOUT_OPTION | CDH_API_TIMOUT_OPTION),
        ERR_INVALID_TIMEOUT_KEY
    );

    let value = fields[1]
        .parse::<u64>()
        .with_context(|| ERR_INVALID_TIMEOUT_PARAM)?;

    Ok(time::Duration::from_secs(value))
}

#[instrument]
fn get_bool_value(param: &str) -> Result<bool> {
    let fields: Vec<&str> = param.split('=').collect();

    if fields.len() != 2 {
        return Ok(false);
    }

    let v = fields[1];

    // first try to parse as bool value
    v.parse::<bool>().or_else(|_err1| {
        // then try to parse as integer value
        v.parse::<u64>().or(Ok(0)).map(|v| !matches!(v, 0))
    })
}

// Return the value from a "name=value" string.
//
// Note:
//
// - A name *and* a value is required.
// - A value can contain any number of equal signs.
// - We could/should maybe check if the name is pure whitespace
//   since this is considered to be invalid.
#[instrument]
fn get_string_value(param: &str) -> Result<String> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() >= 2, ERR_INVALID_GET_VALUE_PARAM);

    // We need name (but the value can be blank)
    ensure!(!fields[0].is_empty(), ERR_INVALID_GET_VALUE_NO_NAME);

    let value = fields[1..].join("=");
    ensure!(!value.is_empty(), ERR_INVALID_GET_VALUE_NO_VALUE);

    Ok(value)
}

#[instrument]
fn get_container_pipe_size(param: &str) -> Result<i32> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() == 2, ERR_INVALID_CONTAINER_PIPE_SIZE);

    let key = fields[0];
    ensure!(
        key == CONTAINER_PIPE_SIZE_OPTION,
        ERR_INVALID_CONTAINER_PIPE_SIZE_KEY
    );

    let value = fields[1]
        .parse::<i32>()
        .with_context(|| ERR_INVALID_CONTAINER_PIPE_SIZE_PARAM)?;

    ensure!(value >= 0, ERR_INVALID_CONTAINER_PIPE_NEGATIVE);

    Ok(value)
}

#[instrument]
fn get_url_value(param: &str) -> Result<String> {
    let value = get_string_value(param)?;
    Ok(Url::parse(&value)?.to_string())
}

#[instrument]
fn get_guest_components_features_value(param: &str) -> Result<GuestComponentsFeatures> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() >= 2, ERR_INVALID_GET_VALUE_PARAM);
    // We need name (but the value can be blank)
    ensure!(!fields[0].is_empty(), ERR_INVALID_GET_VALUE_NO_NAME);
    let value = fields[1..].join("=");
    GuestComponentsFeatures::from_str(&value)
        .map_err(|_| anyhow!(ERR_INVALID_GUEST_COMPONENTS_REST_API_VALUE))
}

#[instrument]
fn get_guest_components_procs_value(param: &str) -> Result<GuestComponentsProcs> {
    let fields: Vec<&str> = param.split('=').collect();
    ensure!(fields.len() >= 2, ERR_INVALID_GET_VALUE_PARAM);

    // We need name (but the value can be blank)
    ensure!(!fields[0].is_empty(), ERR_INVALID_GET_VALUE_NO_NAME);

    let value = fields[1..].join("=");
    GuestComponentsProcs::from_str(&value)
        .map_err(|_| anyhow!(ERR_INVALID_GUEST_COMPONENTS_PROCS_VALUE))
}

#[cfg(test)]
mod tests {
    use test_utils::assert_result;

    use super::*;
    use anyhow::anyhow;
    use serial_test::serial;
    use std::fs::File;
    use std::io::Write;
    use std::time;
    use tempfile::tempdir;

    #[test]
    fn test_new() {
        let config: AgentConfig = Default::default();
        assert!(!config.debug_console);
        assert!(!config.dev_mode);
        assert_eq!(config.log_level, DEFAULT_LOG_LEVEL);
        assert_eq!(config.hotplug_timeout, DEFAULT_HOTPLUG_TIMEOUT);
        #[cfg(feature = "guest-pull")]
        {
            assert!(!config.enable_signature_verification);
            assert_eq!(config.image_policy_file, "");
        }
    }

    #[test]
    // Run in serial to stop the env set interfering with test_from_cmdline_with_args_overwrites
    #[serial]
    fn test_from_cmdline() {
        const TEST_SERVER_ADDR: &str = "vsock://-1:1024";

        #[derive(Debug)]
        struct TestData<'a> {
            contents: &'a str,
            env_vars: Vec<&'a str>,
            debug_console: bool,
            dev_mode: bool,
            log_level: slog::Level,
            hotplug_timeout: time::Duration,
            container_pipe_size: i32,
            server_addr: &'a str,
            unified_cgroup_hierarchy: bool,
            tracing: bool,
            https_proxy: &'a str,
            no_proxy: &'a str,
            guest_components_rest_api: GuestComponentsFeatures,
            guest_components_procs: GuestComponentsProcs,
            #[cfg(feature = "guest-pull")]
            image_registry_auth: &'a str,
            secure_storage_integrity: bool,
            #[cfg(feature = "guest-pull")]
            enable_signature_verification: bool,
            #[cfg(feature = "guest-pull")]
            image_policy_file: &'a str,
        }

        impl Default for TestData<'_> {
            fn default() -> Self {
                TestData {
                    contents: "",
                    env_vars: Vec::new(),
                    debug_console: false,
                    dev_mode: false,
                    log_level: DEFAULT_LOG_LEVEL,
                    hotplug_timeout: DEFAULT_HOTPLUG_TIMEOUT,
                    container_pipe_size: DEFAULT_CONTAINER_PIPE_SIZE,
                    server_addr: TEST_SERVER_ADDR,
                    unified_cgroup_hierarchy: false,
                    tracing: false,
                    https_proxy: "",
                    no_proxy: "",
                    guest_components_rest_api: GuestComponentsFeatures::default(),
                    guest_components_procs: GuestComponentsProcs::default(),
                    #[cfg(feature = "guest-pull")]
                    image_registry_auth: "",
                    secure_storage_integrity: false,
                    #[cfg(feature = "guest-pull")]
                    enable_signature_verification: false,
                    #[cfg(feature = "guest-pull")]
                    image_policy_file: "",
                }
            }
        }

        let tests = &[
            TestData {
                contents: "agent.debug_consolex agent.devmode",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.debug_console agent.devmodex",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.logx=debug",
                ..Default::default()
            },
            TestData {
                contents: "agent.log=debug",
                log_level: slog::Level::Debug,
                ..Default::default()
            },
            TestData {
                contents: "agent.log=debug",
                env_vars: vec!["KATA_AGENT_LOG_LEVEL=trace"],
                log_level: slog::Level::Trace,
                ..Default::default()
            },
            TestData {
                contents: "",
                ..Default::default()
            },
            TestData {
                contents: "foo",
                ..Default::default()
            },
            TestData {
                contents: "foo bar",
                ..Default::default()
            },
            TestData {
                contents: "foo bar",
                ..Default::default()
            },
            TestData {
                contents: "foo agent bar",
                ..Default::default()
            },
            TestData {
                contents: "foo debug_console agent bar devmode",
                ..Default::default()
            },
            TestData {
                contents: "agent.debug_console",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "   agent.debug_console ",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.debug_console foo",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: " agent.debug_console foo",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.debug_console bar",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.debug_console",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.debug_console ",
                debug_console: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "   agent.devmode ",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode foo",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: " agent.devmode foo",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.devmode bar",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.devmode",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "foo agent.devmode ",
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console",
                debug_console: true,
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.hotplug_timeout=100 systemd.unified_cgroup_hierarchy=a",
                debug_console: true,
                dev_mode: true,
                hotplug_timeout: time::Duration::from_secs(100),
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.hotplug_timeout=0 systemd.unified_cgroup_hierarchy=11",
                debug_console: true,
                dev_mode: true,
                unified_cgroup_hierarchy: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.container_pipe_size=2097152 systemd.unified_cgroup_hierarchy=false",
                debug_console: true,
                dev_mode: true,
                container_pipe_size: 2097152,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.container_pipe_size=100 systemd.unified_cgroup_hierarchy=true",
                debug_console: true,
                dev_mode: true,
                container_pipe_size: 100,
                unified_cgroup_hierarchy: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.container_pipe_size=0 systemd.unified_cgroup_hierarchy=0",
                debug_console: true,
                dev_mode: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.devmode agent.debug_console agent.container_pip_siz=100 systemd.unified_cgroup_hierarchy=1",
                debug_console: true,
                dev_mode: true,
                unified_cgroup_hierarchy: true,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=foo"],
                server_addr: "foo",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=="],
                server_addr: "=",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR==foo"],
                server_addr: "=foo",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=foo=bar=baz="],
                server_addr: "foo=bar=baz=",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=unix:///tmp/foo.socket"],
                server_addr: "unix:///tmp/foo.socket",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=unix://@/tmp/foo.socket"],
                server_addr: "unix://@/tmp/foo.socket",
                ..Default::default()
            },
            // Test that env_var has precedence over the command line (which is the current behaviour)
            TestData {
                contents: "agent.server_addr=unix:///tmp/ignored.socket",
                env_vars: vec!["KATA_AGENT_SERVER_ADDR=unix:///tmp/foo.socket"],
                server_addr: "unix:///tmp/foo.socket",
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_LOG_LEVEL="],
                log_level: DEFAULT_LOG_LEVEL,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_LOG_LEVEL=invalid"],
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_LOG_LEVEL=debug"],
                log_level: slog::Level::Debug,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_LOG_LEVEL=debugger"],
                log_level: DEFAULT_LOG_LEVEL,
                ..Default::default()
            },
            TestData {
                contents: "server_addr=unix:///tmp/foo.socket",
                server_addr: TEST_SERVER_ADDR,
                ..Default::default()
            },
            TestData {
                contents: "agent.server_address=unix:///tmp/foo.socket",
                server_addr: TEST_SERVER_ADDR,
                ..Default::default()
            },
            TestData {
                contents: "agent.server_addr=unix:///tmp/foo.socket",
                server_addr: "unix:///tmp/foo.socket",
                ..Default::default()
            },
            TestData {
                contents: " agent.server_addr=unix:///tmp/foo.socket",
                server_addr: "unix:///tmp/foo.socket",
                ..Default::default()
            },
            TestData {
                contents: " agent.server_addr=unix:///tmp/foo.socket a",
                server_addr: "unix:///tmp/foo.socket",
                ..Default::default()
            },
            TestData {
                contents: "trace",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: ".trace",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.tracer",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trac",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace",
                tracing: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=true",
                tracing: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=false",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=0",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=1",
                tracing: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=a",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=foo",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=.",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.trace=,",
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING="],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=''"],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=0"],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=."],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=,"],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=foo"],
                tracing: false,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=1"],
                tracing: true,
                ..Default::default()
            },
            TestData {
                contents: "",
                env_vars: vec!["KATA_AGENT_TRACING=true"],
                tracing: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.https_proxy=http://proxy.url.com:81/",
                https_proxy: "http://proxy.url.com:81/",
                ..Default::default()
            },
            TestData {
                contents: "agent.https_proxy=http://192.168.1.100:81/",
                https_proxy: "http://192.168.1.100:81/",
                ..Default::default()
            },
            TestData {
                contents: "agent.no_proxy=*.internal.url.com",
                no_proxy: "*.internal.url.com",
                ..Default::default()
            },
            TestData {
                contents: "agent.no_proxy=192.168.1.0/24,172.16.0.0/12",
                no_proxy: "192.168.1.0/24,172.16.0.0/12",
                ..Default::default()
            },
            TestData {
               contents: "agent.guest_components_rest_api=attestation",
               guest_components_rest_api: GuestComponentsFeatures::Attestation,
                ..Default::default()
            },
            TestData {
                contents: "agent.guest_components_rest_api=resource",
                guest_components_rest_api: GuestComponentsFeatures::Resource,
                ..Default::default()
            },
            TestData {
                contents: "agent.guest_components_rest_api=all",
                guest_components_rest_api: GuestComponentsFeatures::All,
                ..Default::default()
            },
            TestData {
               contents: "agent.guest_components_procs=api-server-rest",
               guest_components_procs: GuestComponentsProcs::ApiServerRest,
                ..Default::default()
            },
            TestData {
                contents: "agent.guest_components_procs=confidential-data-hub",
                guest_components_procs: GuestComponentsProcs::ConfidentialDataHub,
                ..Default::default()
            },
            TestData {
                contents: "agent.guest_components_procs=attestation-agent",
                guest_components_procs: GuestComponentsProcs::AttestationAgent,
                ..Default::default()
            },
            TestData {
                contents: "agent.guest_components_procs=none",
                guest_components_procs: GuestComponentsProcs::None,
                ..Default::default()
            },
            #[cfg(feature = "guest-pull")]
            TestData {
                contents: "agent.image_registry_auth=file:///root/.docker/config.json",
                image_registry_auth: "file:///root/.docker/config.json",
                ..Default::default()
            },
            #[cfg(feature = "guest-pull")]
            TestData {
                contents: "agent.image_registry_auth=kbs:///default/credentials/test",
                image_registry_auth: "kbs:///default/credentials/test",
                ..Default::default()
            },
            TestData {
                contents: "",
                secure_storage_integrity: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.secure_storage_integrity=true",
                secure_storage_integrity: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.secure_storage_integrity=false",
                secure_storage_integrity: false,
                ..Default::default()
            },
            TestData {
                contents: "agent.secure_storage_integrity=1",
                secure_storage_integrity: true,
                ..Default::default()
            },
            TestData {
                contents: "agent.secure_storage_integrity=0",
                secure_storage_integrity: false,
                ..Default::default()
            },
            #[cfg(feature = "guest-pull")]
            TestData {
                contents: "agent.enable_signature_verification=true",
                enable_signature_verification: true,
                ..Default::default()
            },
            #[cfg(feature = "guest-pull")]
            TestData {
                contents: "agent.image_policy_file=kbs:///default/image-policy/test",
                image_policy_file: "kbs:///default/image-policy/test",
                ..Default::default()
            },
            #[cfg(feature = "guest-pull")]
            TestData {
                contents: "agent.image_policy_file=file:///etc/image-policy.json",
                image_policy_file: "file:///etc/image-policy.json",
                ..Default::default()
            },
        ];

        let dir = tempdir().expect("failed to create tmpdir");

        // Now, test various combinations of file contents and environment
        // variables.
        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let file_path = dir.path().join("cmdline");

            let filename = file_path.to_str().expect("failed to create filename");

            let mut file =
                File::create(filename).unwrap_or_else(|_| panic!("{}: failed to create file", msg));

            file.write_all(d.contents.as_bytes())
                .unwrap_or_else(|_| panic!("{}: failed to write file contents", msg));

            let mut vars_to_unset = Vec::new();

            for v in &d.env_vars {
                let fields: Vec<&str> = v.split('=').collect();

                let name = fields[0];
                let value = fields[1..].join("=");

                env::set_var(name, value);

                vars_to_unset.push(name);
            }

            let config =
                AgentConfig::from_cmdline(filename, vec![]).expect("Failed to parse command line");

            assert_eq!(d.debug_console, config.debug_console, "{}", msg);
            assert_eq!(d.dev_mode, config.dev_mode, "{}", msg);
            assert_eq!(
                d.unified_cgroup_hierarchy, config.unified_cgroup_hierarchy,
                "{}",
                msg
            );
            assert_eq!(d.log_level, config.log_level, "{}", msg);
            assert_eq!(d.hotplug_timeout, config.hotplug_timeout, "{}", msg);
            assert_eq!(d.container_pipe_size, config.container_pipe_size, "{}", msg);
            assert_eq!(d.server_addr, config.server_addr, "{}", msg);
            assert_eq!(d.tracing, config.tracing, "{}", msg);
            assert_eq!(d.https_proxy, config.https_proxy, "{}", msg);
            assert_eq!(d.no_proxy, config.no_proxy, "{}", msg);
            assert_eq!(
                d.guest_components_rest_api, config.guest_components_rest_api,
                "{}",
                msg
            );
            assert_eq!(
                d.guest_components_procs, config.guest_components_procs,
                "{}",
                msg
            );
            #[cfg(feature = "guest-pull")]
            {
                assert_eq!(d.image_registry_auth, config.image_registry_auth, "{}", msg);
                assert_eq!(
                    d.enable_signature_verification, config.enable_signature_verification,
                    "{}",
                    msg
                );
                assert_eq!(d.image_policy_file, config.image_policy_file, "{}", msg);
            }
            assert_eq!(
                d.secure_storage_integrity, config.secure_storage_integrity,
                "{}",
                msg
            );
            for v in vars_to_unset {
                env::remove_var(v);
            }
        }
    }

    #[test]
    // Run in serial to stop the env set interfering with test_from_cmdline
    #[serial]
    fn test_from_cmdline_with_args_overwrites() {
        let expected = AgentConfig {
            dev_mode: true,
            server_addr: "unix:///tmp/overwrite.socket".to_string(),
            ..Default::default()
        };

        let example_config_file_contents =
            "dev_mode = true\nserver_addr = 'unix:///tmp/ignored.socket'";
        let dir = tempdir().expect("failed to create tmpdir");
        let file_path = dir.path().join("config.toml");
        let filename = file_path.to_str().expect("failed to create filename");
        let mut file = File::create(filename).unwrap_or_else(|_| panic!("failed to create file"));
        file.write_all(example_config_file_contents.as_bytes())
            .unwrap_or_else(|_| panic!("failed to write file contents"));

        // Ensure that the env has precedence over agent config file
        env::set_var("KATA_AGENT_SERVER_ADDR", "unix:///tmp/overwrite.socket");

        let config =
            AgentConfig::from_cmdline("", vec!["--config".to_string(), filename.to_string()])
                .expect("Failed to parse command line");

        env::remove_var("KATA_AGENT_SERVER_ADDR");

        assert_eq!(expected.debug_console, config.debug_console);
        assert_eq!(expected.dev_mode, config.dev_mode);
        assert_eq!(
            expected.unified_cgroup_hierarchy,
            config.unified_cgroup_hierarchy,
        );
        assert_eq!(expected.log_level, config.log_level);
        assert_eq!(expected.hotplug_timeout, config.hotplug_timeout);
        assert_eq!(expected.container_pipe_size, config.container_pipe_size);
        assert_eq!(expected.server_addr, config.server_addr);
        assert_eq!(expected.tracing, config.tracing);
    }

    #[test]
    fn test_logrus_to_slog_level() {
        #[derive(Debug)]
        struct TestData<'a> {
            logrus_level: &'a str,
            result: Result<slog::Level>,
        }

        let tests = &[
            TestData {
                logrus_level: "",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL)),
            },
            TestData {
                logrus_level: "foo",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL)),
            },
            TestData {
                logrus_level: "debugging",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL)),
            },
            TestData {
                logrus_level: "xdebug",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL)),
            },
            TestData {
                logrus_level: "trace",
                result: Ok(slog::Level::Trace),
            },
            TestData {
                logrus_level: "debug",
                result: Ok(slog::Level::Debug),
            },
            TestData {
                logrus_level: "info",
                result: Ok(slog::Level::Info),
            },
            TestData {
                logrus_level: "warn",
                result: Ok(slog::Level::Warning),
            },
            TestData {
                logrus_level: "warning",
                result: Ok(slog::Level::Warning),
            },
            TestData {
                logrus_level: "error",
                result: Ok(slog::Level::Error),
            },
            TestData {
                logrus_level: "critical",
                result: Ok(slog::Level::Critical),
            },
            TestData {
                logrus_level: "fatal",
                result: Ok(slog::Level::Critical),
            },
            TestData {
                logrus_level: "panic",
                result: Ok(slog::Level::Critical),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = logrus_to_slog_level(d.logrus_level);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_log_level() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<slog::Level>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_PARAM)),
            },
            TestData {
                param: "=",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "x=",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "=y",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "==",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_PARAM)),
            },
            TestData {
                param: "= =",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_PARAM)),
            },
            TestData {
                param: "x=y",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "agent=debug",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "agent.logg=debug",
                result: Err(anyhow!(ERR_INVALID_LOG_LEVEL_KEY)),
            },
            TestData {
                param: "agent.log=trace",
                result: Ok(slog::Level::Trace),
            },
            TestData {
                param: "agent.log=debug",
                result: Ok(slog::Level::Debug),
            },
            TestData {
                param: "agent.log=info",
                result: Ok(slog::Level::Info),
            },
            TestData {
                param: "agent.log=warn",
                result: Ok(slog::Level::Warning),
            },
            TestData {
                param: "agent.log=warning",
                result: Ok(slog::Level::Warning),
            },
            TestData {
                param: "agent.log=error",
                result: Ok(slog::Level::Error),
            },
            TestData {
                param: "agent.log=critical",
                result: Ok(slog::Level::Critical),
            },
            TestData {
                param: "agent.log=fatal",
                result: Ok(slog::Level::Critical),
            },
            TestData {
                param: "agent.log=panic",
                result: Ok(slog::Level::Critical),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_log_level(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_timeout() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<time::Duration>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_TIMEOUT)),
            },
            TestData {
                param: "agent.hotplug_timeout",
                result: Err(anyhow!(ERR_INVALID_TIMEOUT)),
            },
            TestData {
                param: "foo=bar",
                result: Err(anyhow!(ERR_INVALID_TIMEOUT_KEY)),
            },
            TestData {
                param: "agent.hotplug_timeot=1",
                result: Err(anyhow!(ERR_INVALID_TIMEOUT_KEY)),
            },
            TestData {
                param: "agent.chd_api_timeout=1",
                result: Err(anyhow!(ERR_INVALID_TIMEOUT_KEY)),
            },
            TestData {
                param: "agent.hotplug_timeout=1",
                result: Ok(time::Duration::from_secs(1)),
            },
            TestData {
                param: "agent.hotplug_timeout=3",
                result: Ok(time::Duration::from_secs(3)),
            },
            TestData {
                param: "agent.hotplug_timeout=3600",
                result: Ok(time::Duration::from_secs(3600)),
            },
            TestData {
                param: "agent.cdh_api_timeout=600",
                result: Ok(time::Duration::from_secs(600)),
            },
            TestData {
                param: "agent.hotplug_timeout=0",
                result: Ok(time::Duration::from_secs(0)),
            },
            TestData {
                param: "agent.hotplug_timeout=-1",
                result: Err(anyhow!(
                    "unable to parse timeout

Caused by:
    invalid digit found in string"
                )),
            },
            TestData {
                param: "agent.hotplug_timeout=4jbsdja",
                result: Err(anyhow!(
                    "unable to parse timeout

Caused by:
    invalid digit found in string"
                )),
            },
            TestData {
                param: "agent.hotplug_timeout=foo",
                result: Err(anyhow!(
                    "unable to parse timeout

Caused by:
    invalid digit found in string"
                )),
            },
            TestData {
                param: "agent.hotplug_timeout=j",
                result: Err(anyhow!(
                    "unable to parse timeout

Caused by:
    invalid digit found in string"
                )),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_timeout(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_container_pipe_size() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<i32>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_CONTAINER_PIPE_SIZE)),
            },
            TestData {
                param: "agent.container_pipe_size",
                result: Err(anyhow!(ERR_INVALID_CONTAINER_PIPE_SIZE)),
            },
            TestData {
                param: "foo=bar",
                result: Err(anyhow!(ERR_INVALID_CONTAINER_PIPE_SIZE_KEY)),
            },
            TestData {
                param: "agent.container_pip_siz=1",
                result: Err(anyhow!(ERR_INVALID_CONTAINER_PIPE_SIZE_KEY)),
            },
            TestData {
                param: "agent.container_pipe_size=1",
                result: Ok(1),
            },
            TestData {
                param: "agent.container_pipe_size=3",
                result: Ok(3),
            },
            TestData {
                param: "agent.container_pipe_size=2097152",
                result: Ok(2097152),
            },
            TestData {
                param: "agent.container_pipe_size=0",
                result: Ok(0),
            },
            TestData {
                param: "agent.container_pipe_size=-1",
                result: Err(anyhow!(ERR_INVALID_CONTAINER_PIPE_NEGATIVE)),
            },
            TestData {
                param: "agent.container_pipe_size=foobar",
                result: Err(anyhow!(
                    "unable to parse container pipe size

Caused by:
    invalid digit found in string"
                )),
            },
            TestData {
                param: "agent.container_pipe_size=j",
                result: Err(anyhow!(
                    "unable to parse container pipe size

Caused by:
    invalid digit found in string",
                )),
            },
            TestData {
                param: "agent.container_pipe_size=4jbsdja",
                result: Err(anyhow!(
                    "unable to parse container pipe size

Caused by:
    invalid digit found in string"
                )),
            },
            TestData {
                param: "agent.container_pipe_size=4294967296",
                result: Err(anyhow!(
                    "unable to parse container pipe size

Caused by:
    number too large to fit in target type"
                )),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_container_pipe_size(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_string_value() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<String>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_PARAM)),
            },
            TestData {
                param: "=",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "==",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "x=",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_VALUE)),
            },
            TestData {
                param: "x==",
                result: Ok("=".into()),
            },
            TestData {
                param: "x===",
                result: Ok("==".into()),
            },
            TestData {
                param: "x==x",
                result: Ok("=x".into()),
            },
            TestData {
                param: "x=x",
                result: Ok("x".into()),
            },
            TestData {
                param: "x=x=",
                result: Ok("x=".into()),
            },
            TestData {
                param: "x=x=x",
                result: Ok("x=x".into()),
            },
            TestData {
                param: "foo=bar",
                result: Ok("bar".into()),
            },
            TestData {
                param: "x= =",
                result: Ok(" =".into()),
            },
            TestData {
                param: "x= =",
                result: Ok(" =".into()),
            },
            TestData {
                param: "x= = ",
                result: Ok(" = ".into()),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_string_value(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_guest_components_features_value() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<GuestComponentsFeatures>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_PARAM)),
            },
            TestData {
                param: "=",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "==",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "x=all",
                result: Ok(GuestComponentsFeatures::All),
            },
            TestData {
                param: "x=attestation",
                result: Ok(GuestComponentsFeatures::Attestation),
            },
            TestData {
                param: "x=resource",
                result: Ok(GuestComponentsFeatures::Resource),
            },
            TestData {
                param: "x===",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_REST_API_VALUE)),
            },
            TestData {
                param: "x==x",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_REST_API_VALUE)),
            },
            TestData {
                param: "x=x",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_REST_API_VALUE)),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_guest_components_features_value(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_get_guest_components_procs_value() {
        #[derive(Debug)]
        struct TestData<'a> {
            param: &'a str,
            result: Result<GuestComponentsProcs>,
        }

        let tests = &[
            TestData {
                param: "",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_PARAM)),
            },
            TestData {
                param: "=",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "==",
                result: Err(anyhow!(ERR_INVALID_GET_VALUE_NO_NAME)),
            },
            TestData {
                param: "x=attestation-agent",
                result: Ok(GuestComponentsProcs::AttestationAgent),
            },
            TestData {
                param: "x=confidential-data-hub",
                result: Ok(GuestComponentsProcs::ConfidentialDataHub),
            },
            TestData {
                param: "x=none",
                result: Ok(GuestComponentsProcs::None),
            },
            TestData {
                param: "x=api-server-rest",
                result: Ok(GuestComponentsProcs::ApiServerRest),
            },
            TestData {
                param: "x===",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_PROCS_VALUE)),
            },
            TestData {
                param: "x==x",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_PROCS_VALUE)),
            },
            TestData {
                param: "x=x",
                result: Err(anyhow!(ERR_INVALID_GUEST_COMPONENTS_PROCS_VALUE)),
            },
        ];

        for (i, d) in tests.iter().enumerate() {
            let msg = format!("test[{}]: {:?}", i, d);

            let result = get_guest_components_procs_value(d.param);

            let msg = format!("{}: result: {:?}", msg, result);

            assert_result!(d.result, result, msg);
        }
    }

    #[test]
    fn test_config_builder_from_string() {
        let config = AgentConfig::from_str(
            r#"
               dev_mode = true
               server_addr = 'vsock://8:2048'
               guest_components_procs = "api-server-rest"
               guest_components_rest_api = "all"
              "#,
        )
        .unwrap();

        // Verify that the override worked
        assert!(config.dev_mode);
        assert_eq!(config.server_addr, "vsock://8:2048");
        assert_eq!(
            config.guest_components_procs,
            GuestComponentsProcs::ApiServerRest
        );
        assert_eq!(
            config.guest_components_rest_api,
            GuestComponentsFeatures::All
        );

        // Verify that the default values are valid
        assert_eq!(config.hotplug_timeout, DEFAULT_HOTPLUG_TIMEOUT);
    }
}
