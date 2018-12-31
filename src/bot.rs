use cbor;
use irc::client::prelude::*;
use markov_chain::Chain;
use rand::{self, Rng};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};

type UserSettingsMap = HashMap<String, HashMap<String, UserSettings>>;
type ChainMap = HashMap<String, HashMap<String, Chain<String>>>;

const DEFAULT_CHANCE: f64 = 0.01;
const DEFAULT_ORDER: usize = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UserSettings {
    pub ignore: bool,
    pub chance: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlobFile {
    chains: ChainMap,
    user_settings: UserSettingsMap,
    order: usize,
}

pub struct IrcBot {
    chains: ChainMap,
    allchains: HashMap<String, Chain<String>>,
    user_settings: UserSettingsMap,
    ignore: Vec<String>,
    order: usize,
    chance: f64,
    server: IrcServer,
}

impl IrcBot {
    pub fn new(server: IrcServer, options: HashMap<String, String>) -> Self {
        IrcBot {
            chains: HashMap::new(),
            allchains: HashMap::new(),
            user_settings: HashMap::new(),
            ignore: options
                .get("ignore")
                .map(|x| x.split(',').map(str::to_string).collect())
                .unwrap_or(vec![]),
            order: options
                .get("order")
                .map(|x| x.parse::<usize>().unwrap())
                .unwrap_or(DEFAULT_ORDER),
            chance: options
                .get("chance")
                .map(|x| x.parse::<f64>().unwrap())
                .unwrap_or(DEFAULT_CHANCE),
            server,
        }
    }

    /// Constructs this IrcBot with a pre-saved chain and user settings.
    pub fn from_blob_file(
        server: IrcServer,
        options: HashMap<String, String>,
        blob: BlobFile,
    ) -> Self {
        IrcBot {
            chains: blob.chains,
            allchains: HashMap::new(),
            user_settings: blob.user_settings,
            ignore: options
                .get("ignore")
                .map(|x| x.split(',').map(str::to_string).collect())
                .unwrap_or(vec![]),
            chance: options
                .get("chance")
                .map(|x| x.parse::<f64>().unwrap())
                .unwrap_or(DEFAULT_CHANCE),
            order: blob.order,
            server,
        }
    }

    /// Handles an incoming IRC message.
    pub fn handle(&mut self, msg: Message) {
        match msg.command {
            Command::PRIVMSG(ref channel, ref msg_str) => {
                if let Some(prefix) = msg.prefix {
                    self.channel_message(&prefix.split('!').nth(0).unwrap(), channel, msg_str);
                }
            }
            _ => trace!("not handled: {}", msg),
        }
    }

    /// Handles a channel message.
    fn channel_message(&mut self, sender: &str, channel: &str, msg: &str) {
        // ignore messages from ourself
        if sender == self.server.current_nickname() {
            return;
        }

        let msg_parts = msg.split_whitespace().collect::<Vec<_>>();
        // handle markov command
        if msg_parts.len() > 1 && msg_parts[0] == "!markov" {
            self.handle_command(sender, channel, &msg_parts);
        } else if !self.is_ignored(channel, sender) {
            let chance = { self.user_settings_mut(channel, sender).chance };
            // Train the allchain first
            // if we train it second, it's possible it may not have been constructed yet, and we double-train it as a result
            {
                let allchain = self.allchain_mut(channel);
                allchain.train_string(msg);
            }
            // Train the user's chain
            {
                let chain = self.user_chain_mut(channel, sender);
                chain.train_string(msg);
            }

            // Reply if we feel like it
            let random = rand::thread_rng().next_f64();
            if random < chance {
                let generated = { self.user_chain_mut(channel, sender).generate_sentence() };
                let message = format!("{}: {}", sender, generated);
                if let Err(e) = self.server.send_privmsg(channel, &message) {
                    error!("{}", e);
                }
            }
        }
    }

    fn allchain_mut(&mut self, channel: &str) -> &mut Chain<String> {
        if !self.allchains.contains_key(channel) {
            debug!("building allchain for {}", channel);
            let mut allchain = Chain::new(self.order);
            if self.chains.get(channel).is_none() {
                self.chains.insert(channel.to_string(), HashMap::new());
            } else {
                for (_, ref chain) in self.chains.get(channel).unwrap() {
                    allchain.merge(chain);
                }
            }
            self.allchains.insert(channel.to_string(), allchain);
        }
        self.allchains.get_mut(channel).unwrap()
    }

    fn user_chain_mut(&mut self, channel: &str, user: &str) -> &mut Chain<String> {
        if !self.chains.contains_key(channel) {
            self.chains.insert(channel.to_string(), HashMap::new());
        }
        let channel = self.chains.get_mut(channel).unwrap();

        if !channel.contains_key(user) {
            channel.insert(user.to_string(), Chain::new(self.order));
        }
        channel.get_mut(user).unwrap()
    }

    fn user_settings_mut(&mut self, channel: &str, user: &str) -> &mut UserSettings {
        if !self.user_settings.contains_key(channel) {
            self.user_settings
                .insert(channel.to_string(), HashMap::new());
        }
        let channel = self.user_settings.get_mut(channel).unwrap();

        if !channel.contains_key(user) {
            channel.insert(
                user.to_string(),
                UserSettings {
                    ignore: false,
                    chance: self.chance,
                },
            );
        }
        channel.get_mut(user).unwrap()
    }

    /// Gets whether a user on a given channel is ignored
    fn is_ignored(&self, channel: &str, user: &str) -> bool {
        self.ignore
            .iter()
            .map(String::as_str)
            .find(|&f| f == user)
            .is_some()
            || self
                .user_settings
                .get(channel)
                .map(|c| c.get(user).map(|u| u.ignore).unwrap_or(false))
                .unwrap_or(false)
    }

    fn handle_command(&mut self, sender: &str, channel: &str, parts: &[&str]) {
        assert_eq!(parts[0], "!markov");
        assert!(parts.len() > 1);

        match parts[1] {
            "emulate" => {
                if parts.len() < 3 {
                    if let Err(e) = self
                        .server
                        .send_privmsg(channel, "Usage: !markov emulate <user> [<channel>]")
                    {
                        error!("{}", e);
                    }
                } else {
                    let (user, chan) = match (parts.get(2), parts.get(3)) {
                        (Some(user), Some(channel)) => (user, channel), // user and channel
                        (Some(user), None) => (user, &channel),         // user no channel
                        (_, _) => {
                            if let Err(e) = self
                                .server
                                .send_privmsg(channel, "Usage: !markov emulate <user> [<channel>]")
                            {
                                error!("{}", e);
                            };
                            return;
                        }
                    };
                    if let Some(chan_chain) = self.chains.get(chan.to_string()) {
                        if let Some(user_chain) = chan_chain.get(user.to_string()) {
                            if !chain.is_empty() {
                                let gen = chain.generate_sentence();
                                let message = format!("{}: {}", sender, gen);
                                if let Err(e) = self.server.send_privmsg(channel, &message) {
                                    error!("{}", e);
                                }
                            }
                        } else {
                            let message = format!("{}: No chain for user {}", sender, user);
                            if let Err(e) = self.server.send_privmsg(channel, &message) {
                                error!("{}", e);
                            }
                        }
                    } else {
                        let message = format!("{}: No chain for channel {}", sender, chan);
                        if let Err(e) = self.server.send_privmsg(channel, &message) {
                            error!("{}", e);
                        }
                    }
                }
            }
            "force" => {
                let chain = self
                    .chains
                    .entry(channel.to_string())
                    .or_insert(HashMap::new())
                    .entry(sender.to_string())
                    .or_insert(Chain::new(self.order));
                if !chain.is_empty() {
                    let gen = chain.generate_sentence();
                    let message = format!("{}: {}", sender, gen);
                    if let Err(e) = self.server.send_privmsg(channel, &message) {
                        error!("{}", e);
                    }
                }
            }
            "all" => {
                {
                    self.allchain_mut(channel);
                } // this will initialize the allchain if necessary
                if let Some(chain) = self.allchains.get(channel) {
                    if !chain.is_empty() {
                        let gen = chain.generate_sentence();
                        let message = format!("{}: {}", sender, gen);
                        if let Err(e) = self.server.send_privmsg(channel, &message) {
                            error!("{}", e);
                        }
                    }
                }
            }
            "ignore" => {
                if !self.is_ignored(channel, sender) {
                    {
                        let user_settings = self.user_settings_mut(channel, sender);
                        user_settings.ignore = false;
                    }
                    if let Err(e) = self.server.send_privmsg(
                        sender,
                        "You are now being ignored. Use !markov listen to undo this command",
                    ) {
                        error!("{}", e);
                    }
                }
            }
            "listen" => {
                if self.is_ignored(channel, sender) {
                    {
                        let user_settings = self.user_settings_mut(channel, sender);
                        user_settings.ignore = false;
                    }
                    if let Err(e) = self.server.send_privmsg(sender, "Markov is now listening to what you say. Use !markov ignore to undo this command.") {
                    error!("{}", e);
                }
                }
            }
            "chance" => {
                let response = if parts.len() <= 2 {
                    let user_settings = self.user_settings_mut(channel, sender);
                    format!("Your markov chance is {}", user_settings.chance)
                } else {
                    if let Ok(chance) = parts[2].parse::<f64>() {
                        if chance <= self.chance && chance >= 0.0 {
                            let user_settings = self.user_settings_mut(channel, sender);
                            user_settings.chance = chance;
                            format!(
                                "Your chance for getting a random message from markov is {}",
                                chance
                            )
                        } else {
                            format!(
                                "The chance mut be set to a valid number between 0.0 and {}",
                                self.chance
                            )
                        }
                    } else {
                        format!("Invalid number format")
                    }
                };
                if let Err(e) = self.server.send_privmsg(sender, &response) {
                    error!("{}", e);
                }
            }
            "status" => {
                let user_total = { Self::get_chain_total(self.user_chain_mut(channel, sender)) };
                let all_total = { Self::get_chain_total(self.allchain_mut(channel)) };
                let status = ((user_total as f64) / (all_total as f64)) * 100.0;
                let message = format!("{}: You are worth {:.4}% of the channel", sender, status);
                if let Err(e) = self.server.send_privmsg(channel, &message) {
                    error!("{}", e);
                }
            }
            _ => {}
        }
    }

    fn get_chain_total(chain: &Chain<String>) -> u32 {
        chain
            .chain()
            .iter()
            .map(|(_, link)| link.iter().fold(0, |a, (_, weight)| a + weight))
            .fold(0, |a, b| a + b)
    }

    /// Saves a blob of the chains and user settings.
    pub fn save_blob(&mut self, path: &str) -> io::Result<()> {
        info!("saving chains");
        let save_data = BlobFile {
            chains: self.chains.clone(),
            user_settings: self.user_settings.clone(),
            order: self.order,
        };
        let cbor_out = cbor::to_vec(&save_data).unwrap();
        let mut file = OpenOptions::new().write(true).create(true).open(path)?;
        file.write_all(&cbor_out)
    }

    /// Reads a blob of chains and user settings.
    pub fn read_blob(path: &str) -> io::Result<BlobFile> {
        debug!("reading from {}", path);
        let mut file = OpenOptions::new().read(true).open(path)?;
        let mut cbor_in = Vec::new();
        file.read_to_end(&mut cbor_in)?;

        let read_data = cbor::from_slice::<BlobFile>(&cbor_in)
            .expect(&format!("invalid cbor data in {}", path));
        for (_, ref c_chain) in read_data.chains.iter() {
            for (_, ref u_chain) in c_chain.iter() {
                assert_eq!(u_chain.order(), read_data.order);
            }
        }
        trace!("Read data: {:?}", &read_data);
        Ok(read_data)
    }
}
