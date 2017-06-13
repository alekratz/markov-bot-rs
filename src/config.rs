use toml;
use std::fs::File;
use std::io::Read;
use std::result;
use std::collections::HashMap;

type Result<T> = result::Result<T, String>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProgramConfig {
    pub servers: HashMap<String, Server>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Server {
    pub address: String,
    pub port: Option<u16>,
    pub ssl: Option<bool>,
    pub accept_invalid_certs: Option<bool>,
    pub nick: String,
    pub user: Option<String>,
    pub ignore: Option<Vec<String>>,
    pub channels: Vec<Channel>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub key: Option<String>,
    pub ignore: Option<Vec<String>>,
}


