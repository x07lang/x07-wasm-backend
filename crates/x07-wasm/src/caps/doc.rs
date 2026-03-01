use std::net::IpAddr;

use ipnet::IpNet;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CapabilitiesDoc {
    pub fs: FsCaps,
    pub env: EnvCaps,
    pub secrets: SecretsCaps,
    pub network: NetworkCaps,
    pub clocks: ModeCaps,
    pub random: ModeCaps,
}

#[derive(Debug, Clone)]
pub struct NetworkDeny {
    pub code: &'static str,
    pub message: String,
}

impl CapabilitiesDoc {
    pub fn network_check(&self, scheme: &str, host: &str, port: u16) -> Result<(), NetworkDeny> {
        if self.network.mode == NetworkMode::Deny {
            return Err(NetworkDeny {
                code: "X07WASM_CAPS_NET_DENIED",
                message: "wasi:http send_request denied: network.mode=deny".to_string(),
            });
        }

        let want_proto = match scheme {
            "http" => NetworkProto::Http,
            "https" => NetworkProto::Https,
            _ => {
                return Err(NetworkDeny {
                    code: "X07WASM_CAPS_NET_DENIED",
                    message: format!(
                        "wasi:http send_request denied: unsupported scheme {scheme:?}"
                    ),
                })
            }
        };

        if !self
            .network
            .allowlist
            .iter()
            .any(|e| e.proto == want_proto && e.port == port && e.host.eq_ignore_ascii_case(host))
        {
            return Err(NetworkDeny {
                code: "X07WASM_CAPS_NET_DENIED",
                message: format!(
                    "wasi:http send_request denied by capabilities: {}://{}:{}",
                    scheme, host, port
                ),
            });
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if self.network.hardening.allows_ip_literal(ip) {
                return Ok(());
            }

            if self.network.hardening.deny_private_ips && is_private_or_reserved_ip(ip) {
                return Err(NetworkDeny {
                    code: "X07WASM_CAPS_NET_PRIVATE_IP_DENIED",
                    message: format!("wasi:http send_request denied by hardening: private ip {ip}"),
                });
            }

            if self.network.hardening.deny_ip_literals {
                return Err(NetworkDeny {
                    code: "X07WASM_CAPS_NET_IP_LITERAL_DENIED",
                    message: format!("wasi:http send_request denied by hardening: ip literal {ip}"),
                });
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsCaps {
    #[serde(default)]
    pub preopens: Vec<FsPreopen>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsPreopen {
    pub path: String,
    pub mode: FsPreopenMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FsPreopenMode {
    Ro,
    Rw,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnvCaps {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecretsCaps {
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkCaps {
    pub mode: NetworkMode,

    #[serde(default)]
    pub allowlist: Vec<NetworkEndpoint>,

    pub hardening: NetworkHardening,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkHardening {
    pub deny_ip_literals: bool,
    pub deny_private_ips: bool,

    #[serde(default)]
    pub allow_ip_cidrs: Vec<IpNet>,
}

impl NetworkHardening {
    pub fn allows_socket_ip(&self, ip: IpAddr) -> bool {
        if self.allows_ip_literal(ip) {
            return true;
        }
        if self.deny_private_ips && is_private_or_reserved_ip(ip) {
            return false;
        }
        true
    }

    fn allows_ip_literal(&self, ip: IpAddr) -> bool {
        self.allow_ip_cidrs.iter().any(|cidr| cidr.contains(&ip))
    }
}

fn is_private_or_reserved_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Deny,
    Allowlist,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkEndpoint {
    pub host: String,
    pub port: u16,
    pub proto: NetworkProto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkProto {
    Tcp,
    Udp,
    Http,
    Https,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModeCaps {
    pub mode: CapabilityMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMode {
    Deny,
    Record,
    Allow,
}
