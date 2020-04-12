#[macro_use]
extern crate log;

use dotenv::dotenv;
use futures::stream::{self, Stream, StreamExt};
use job_scheduler::{Job, JobScheduler};
use lettre::{
    smtp::{extension::ClientId, ClientSecurity, ConnectionReuseParameters},
    SendableEmail, SmtpClient, SmtpTransport, Transport,
};
use num_cpus;
use std::{
    env,
    sync::{Arc, Mutex},
};
use tokio::runtime::Builder;

use core::{Account, Api, Claim, Comment, Emails, Storage};

fn all_comments(
    api_ref: Arc<Api>,
    page_size_ref: Arc<usize>,
) -> impl Stream<Item = (Account, Claim, Comment)> {
    let claim_api_ref = api_ref.clone();
    let claim_page_ref = page_size_ref.clone();

    let comment_api_ref = api_ref.clone();
    let comment_page_ref = page_size_ref.clone();

    let buffer = num_cpus::get();

    api_ref
        .stream_accounts(*page_size_ref)
        .map(move |account| {
            claim_api_ref
                .stream_claims_by_account_id(account.id.clone(), *claim_page_ref)
                .zip(stream::repeat(account.clone()))
        })
        .flatten()
        .map(|res| async { res })
        .buffer_unordered(buffer)
        .map(move |(claim, account)| {
            comment_api_ref
                .stream_comments_by_claim_id(claim.id.clone(), *comment_page_ref)
                .zip(stream::repeat((claim, account).clone()))
                .map(|(comment, (claim, account))| (account, claim, comment))
        })
        .flatten()
        .map(|res| async { res })
        .buffer_unordered(buffer)
}

fn notify_new_comments(
    api_ref: Arc<Api>,
    storage_ref: Arc<Storage>,
    emails_ref: Arc<Emails>,
    mailer_ref: Arc<Mutex<SmtpTransport>>,
    page_size_ref: Arc<usize>,
) {
    let mut rt = Builder::new()
        .threaded_scheduler()
        .enable_io()
        .build()
        .expect("Unable to create runtime");

    rt.block_on(async {
        info!("Finding new comments");

        all_comments(api_ref, page_size_ref)
            .filter_map(|(account, claim, comment)| async {
                let comment_id = comment.id.to_owned();

                if let Some(comment_entity) = storage_ref.get_comment_by_id(comment_id.clone()) {
                    if &comment_entity.comment != &comment.comment {
                        info!("Comment {} is updated", &comment_id);

                        storage_ref
                            .delete_comment_by_id(comment_id)
                            .expect("Could not delete comment");

                        let new_comment_entity = storage_ref
                            .save_comment(account, claim, comment)
                            .expect("Could not save comment");

                        Some(new_comment_entity)
                    } else {
                        None
                    }
                } else {
                    info!("Logging new comment {}", &comment_id);

                    let new_comment_entity = storage_ref
                        .save_comment(account, claim, comment)
                        .expect("Could not save comment");

                    Some(new_comment_entity)
                }
            })
            .for_each_concurrent(None, |comment_entity| async {
                info!("Sending email for {}", &comment_entity.commenter_name);

                let email: SendableEmail = emails_ref.notification_email(comment_entity).into();

                mailer_ref
                    .lock()
                    .expect("Unable to get lock")
                    .send(email)
                    .expect("Unable to send mail");
            })
            .await;

        info!("Done reading comments");
    });
}

fn main() {
    env_logger::init();
    dotenv().ok();

    info!("Loading config");

    let keys = vec![
        "API_URL".to_string(),
        "DATABASE_URL".to_string(),
        "PAGE_SIZE".to_string(),
        "SMTP_ADDRESS".to_string(),
        "SMTP_FROM".to_string(),
        "SMTP_TO".to_string(),
        "WATCHER_CRON".to_string(),
    ];

    dotenv::vars()
        .filter(|(key, _)| keys.contains(key))
        .for_each(|(key, value)| {
            info!("{} = {}", key, value);
        });

    let api_url = env::var("API_URL").unwrap_or("http://127.0.0.1:5279".to_string());
    let database_url = env::var("DATABASE_URL").unwrap_or("data.db".to_string());
    let page_size = env::var("PAGE_SIZE")
        .unwrap_or("50".to_string())
        .parse::<usize>()
        .unwrap_or(50);
    let smtp_address = env::var("SMTP_ADDRESS").unwrap_or("127.0.0.1:1025".to_string());
    let smtp_from = env::var("SMTP_FROM").unwrap_or("notifier@lbry.local".to_string());
    let smtp_to = env::var("SMTP_TO").unwrap_or("user@lbry.local".to_string());
    let watcher_cron = env::var("WATCHER_CRON").unwrap_or("* 0 * * * *".to_string());

    let storage = Storage::open(database_url.clone()).expect("Unable to connect to database");
    let api = Api::new(api_url.clone());
    let emails = Emails::new(smtp_from, smtp_to);

    let mailer = SmtpClient::new(smtp_address, ClientSecurity::None)
        .expect("Unable to connect to SMTP client")
        .hello_name(ClientId::Domain("localhost".to_string()))
        .smtp_utf8(true)
        .connection_reuse(ConnectionReuseParameters::ReuseUnlimited)
        .transport();

    let storage_ref = Arc::new(storage);
    let api_ref = Arc::new(api);
    let emails_ref = Arc::new(emails);
    let mailer_ref = Arc::new(Mutex::new(mailer));
    let page_size_ref = Arc::new(page_size);

    info!("Starting application");

    let mut sched = JobScheduler::new();
    let watcher_job = Job::new(
        watcher_cron.parse().expect("Unable to create watcher job"),
        || {
            info!("Starting task to notify new comments");

            notify_new_comments(
                api_ref.clone(),
                storage_ref.clone(),
                emails_ref.clone(),
                mailer_ref.clone(),
                page_size_ref.clone(),
            );

            info!("Done task for notifying new comments");
        },
    );

    notify_new_comments(
        api_ref.clone(),
        storage_ref.clone(),
        emails_ref.clone(),
        mailer_ref.clone(),
        page_size_ref.clone(),
    );

    sched.add(watcher_job);

    loop {
        sched.tick();

        std::thread::sleep(sched.time_till_next_job());
    }
}
