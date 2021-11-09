# Overview

A simple server allowing for the hosing of chat-app like functionality.

# Requirements

- [Rust](https://www.rust-lang.org/learn/get-started) (>= `1.52.0`)
- [websocat](https://github.com/vi/websocat) - For testing WebSocket connections

# Deploying

```bash
cargo run --release
```

For development:

```bash
cargo watch -x run --clear --no-gitignore
```

To connect to the server:

```bash
websocat ws://localhost:3030/chat/:name
```

Where `:name` represents the room name to connect to. (e.g. `websocat ws://localhost:3030/chat/public` connects to the 'public' room)

![bi_terminal](https://user-images.githubusercontent.com/59901837/140879765-b46a53f7-ac7f-4f01-8837-bc817b9bd3c1.gif)

# Testing

For running tests, simply do:

```bash
cargo test
```

_This may take some time due to some of the tests that deal with large chunk of total writes to the DB._