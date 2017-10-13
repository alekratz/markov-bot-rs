# markov-bot-rs
An IRC bot that spits out markov chains at you users.

## Usage

I haven't tested this on Windows, but if you can build Rust on Windows, you can probably build this project.

1. Clone.

    `git clone https://github.com/alekratz/markov-bot-rs.git`

2. Copy the example config over and fill it out for your needs.

    `cp markov-bot{.example,}.json`

3. Compile the bot.

    `cargo build`

    OR

    `cargo build --release`

4. Run.
    
    `cargo run`

    OR

    `target/debug/markov-bot-rs`

    OR

    `target/release/markov-bot-rs`
