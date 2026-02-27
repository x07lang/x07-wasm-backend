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

impl CapabilitiesDoc {
    pub fn network_allows(&self, scheme: &str, host: &str, port: u16) -> bool {
        if self.network.mode == NetworkMode::Deny {
            return false;
        }

        let want_proto = match scheme {
            "http" => NetworkProto::Http,
            "https" => NetworkProto::Https,
            _ => return false,
        };

        self.network
            .allowlist
            .iter()
            .any(|e| e.proto == want_proto && e.port == port && e.host.eq_ignore_ascii_case(host))
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
