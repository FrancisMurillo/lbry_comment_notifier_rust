# LBRY Comment Notifier

A small Rust binary polling the LBRY API for new comments via email.
Actually, this is the Rust equivalent of the [elixir
app](https://github.com/FrancisMurillo/lbry_comment_notifier.git) and
much harder to build but more satisfying in learning async.

Every hour, the application fetches all comments by going throught these
three API methods:
[account_list](https://lbry.tech/api/sdk#account_list),
[claim_list](https://lbry.tech/api/sdk#claim_list) and
[comment_list](https://lbry.tech/api/sdk#comment_list). By fetching all
accounts, the claims can be obtained; using those in turn, the comments
can be fetched. Each comment is stored in the persistent storage and new
comment are emailed. Since the APIs are paginated, the algorithm may not
be the most efficient and perhaps LBRY can offer this functionality at a
lower cost. For now though, paginated polling is good enough for a small
channel.

## Installation

As with the Elixir app, this was meant to run locally on a Raspberry Pi
3 with a [configured lbry-sdk as the
API](https://github.com/lbryio/lbry-sdk) (or a [lbry-desktop which hosts
an API too](https://github.com/lbryio/lbry-desktop)) and a [mailcatcher
as a SMTP server](https://github.com/sj26/mailcatcher). So assuming the
default configuration with an `lbry-sdk` running at
`http://localhost:5279` and a `mailcatcher` at `http://0.0.0.0:1080` and
`smtp://0.0.0.0:1025`, the app can be built and run using:

```shell
cargo build --release
RUST_LOG=runner=info,core=debug ./target/release/runner
```

It can also be configured via `dotenv` with a `.env`:

```
# Default values for the .envrc

# URL of the LBRY SDK
API_URL=http://127.0.0.1:5279

# Name of the SQLite3 database
DATABASE_URL=data.db
# Number of records fetched per request when consuming a paginated endpoint
PAGE_SIZE=50

# STMP address of the mailcatcher
SMTP_ADDRESS=127.0.0.1:1025
# From field for the sent email
SMTP_FROM=notifier@lbry.local
# To field for the sent email
SMTP_TO=user@lbry.local

# Cron schedule of the watcher
WATCHER_CRON="* 0 * * * *"
```
