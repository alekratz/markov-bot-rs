extern crate markov_chain;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate ansi_term;
extern crate irc;
extern crate ctrlc;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_cbor as cbor;
extern crate rand;

mod bot;

use bot::IrcBot;

use env_logger::LogBuilder;
use log::{LogRecord, LogLevelFilter, LogLevel};
use ansi_term::{Style, Colour};
use irc::client::prelude::*;

use std::time::Duration;
use std::thread;
use std::env;
use std::process;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

const DEFAULT_CONFIG: &str = "markov-bot.json";

/// Initializes the global logger.
fn init_logger() {
    let logger_format = |record: &LogRecord| {
        let color = match record.level() {
            LogLevel::Error => Colour::Red.bold(),
            LogLevel::Warn => Style::new().fg(Colour::Yellow),
            LogLevel::Info => Style::new().fg(Colour::Green),
            LogLevel::Debug => Style::new().fg(Colour::Blue),
            _ => Colour::White.dimmed(),
        };
        format!("{}", color.paint(format!("[{level:07}] [{location}] {msg}",
                                          location=record.location().module_path(), level=record.level(),
                                          msg=record.args())))
    };
    let mut builder = LogBuilder::new();
    builder.filter(None, LogLevelFilter::Warn);
    if let Ok(env_var) = env::var("RUST_LOG") {
        builder.parse(env_var.as_str());
    }
    builder.format(logger_format)
        .init()
        .unwrap();
}

macro_rules! exit_error {
    ($fmt:expr, $($item:expr),*) => {{
        error!($fmt, $($item),*);
        process::exit(1)
    }};
}

fn run(config: Config) {
    //let mut threads = Vec::new();
    debug!("starting server {}", config.server.as_ref().unwrap());
    let options = config.options
        .as_ref()
        .map(|x| x.clone())
        .unwrap_or(HashMap::new());
    let save_interval = options
        .get("save_interval")
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(3600);
    let chain_file = format!("{}.cbor", options.get("chain_file")
        .map(String::clone)
        .unwrap_or(config.server.clone().unwrap()));
    let server = IrcServer::from_config(config).unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let save_thread;

    // start the server connection and handler thread
    server.identify().unwrap();
    {
        debug!("attempting to read blob file at {}", &chain_file);
        let bot = Arc::new(Mutex::new(
            match IrcBot::read_blob(&chain_file) {
                Ok(blob_file) => {
                    info!("using blob file {}", &chain_file);
                    IrcBot::from_blob_file(server.clone(), options, blob_file)
                },
                Err(e) => {
                    info!("could not read blob file {}: {}", &chain_file, e);
                    info!("one will be created instead");
                    IrcBot::new(server.clone(), options)
                },
            }
        ));
        // Set up the handler thread
        {
            let bot = bot.clone();
            thread::spawn(move || {
                debug!("starting bot thread");
                for msg in server.iter() {
                    match msg {
                        Ok(msg) => {
                            let mut bot = bot.lock()
                                .unwrap();
                            bot.handle(msg)
                        },
                        Err(e) => {
                            error!("{}", e);
                            break;
                        }
                    }
                }
            });
        }
        //threads.push(bot_thread);

        let running = running.clone();
        save_thread = thread::spawn(move || {
            // save every hour
            let bot = bot.clone();
            let ref chain_file = chain_file;
            debug!("starting save thread");
            'outer: while running.load(Ordering::SeqCst) {
                let mut count = 0;
                while count < save_interval * 10 {
                    thread::sleep(Duration::from_millis(100));
                    count += 1;
                    if !running.load(Ordering::SeqCst) {
                        break 'outer;
                    }
                }
                // special bot lock block
                {
                    let mut bot = bot.lock().unwrap();
                    if let Err(write_err) = bot.save_blob(chain_file) {
                        error!("error writing {}: {}", chain_file, write_err);
                    }
                }
            }
            info!("saving one last time");
            // special bot lock block
            {
                let mut bot = bot.lock().unwrap();
                if let Err(write_err) = bot.save_blob(chain_file) {
                    error!("error writing {}: {}", chain_file, write_err);
                }
            }
        });
    }

    debug!("setting ctrlc handler");
    {
        let running = running.clone();
        ctrlc::set_handler(move || {
            info!("ctrl-c caught");
            running.store(false, Ordering::SeqCst);
        }).unwrap();
    }

    info!("main loop");
    while running.load(Ordering::SeqCst) { thread::sleep(Duration::from_millis(1)); }
    info!("joining save thread");
    save_thread.join()
        .unwrap();
}

fn main() {
    init_logger();
    let config_path = DEFAULT_CONFIG;
    trace!("Loading {}", config_path);
    let config = match Config::load(config_path) {
        Ok(c) => c,
        Err(e) => exit_error!("could not load config {}: {}", config_path, e),
    };

    trace!("Starting main server");
    trace!("Config: {:?}", config);
    run(config);
}
