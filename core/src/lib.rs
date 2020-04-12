#[macro_use]
extern crate diesel;

#[macro_use]
extern crate diesel_migrations;

#[macro_use]
extern crate log;

pub mod schema;

use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::prelude::*;
use futures::{
    future::{FutureExt, TryFutureExt},
    prelude::Future,
    stream::{self, FuturesUnordered, Stream, StreamExt},
};
use lettre_email::{Email, EmailBuilder};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, value::Value};

use self::schema::comments;

embed_migrations!("../migrations");

#[derive(Clone, Debug, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Claim {
    #[serde(rename(deserialize = "claim_id"))]
    pub id: String,
    pub name: String,
    #[serde(with = "date_format")]
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Comment {
    #[serde(rename(deserialize = "comment_id"))]
    pub id: String,

    pub claim_id: String,

    pub comment: String,

    #[serde(rename(deserialize = "channel_id"))]
    pub commenter_id: String,
    #[serde(rename(deserialize = "channel_name"))]
    pub commenter_name: String,
    #[serde(rename(deserialize = "channel_url"))]
    pub commenter_url: String,

    pub is_hidden: bool,

    #[serde(with = "date_format")]
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Insertable, Queryable)]
#[table_name = "comments"]
pub struct CommentEntity {
    pub id: String,
    pub account_id: String,
    pub claim_id: String,
    pub claim_name: String,
    pub commenter_id: String,
    pub commenter_name: String,
    pub commenter_url: String,
    pub comment: String,
    pub is_hidden: bool,
    pub timestamp: NaiveDateTime,
}

mod date_format {
    use chrono::{DateTime, TimeZone, Utc};
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let i = i64::deserialize(deserializer)?;
        Ok(Utc.timestamp(i, 0))
    }
}

#[derive(Clone, Debug)]
pub struct Api {
    client: Client,
    url: String,
}

#[derive(Debug)]
pub enum ApiError {
    InvalidResponse,
    NetworkError(reqwest::Error),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Self::InvalidResponse => write!(f, "Invalid response received"),
            Self::NetworkError(ref reqwest_error) => reqwest_error.fmt(f),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ApiPayload<A> {
    result: PaginatedApiResult<A>,
}

#[derive(Debug, Deserialize)]
pub struct PaginatedApiResult<A> {
    items: Vec<A>,
    page: usize,
    page_size: usize,
    total_items: usize,
    total_pages: usize,
}

fn stream_paginated<'r, A: 'r, F: 'r, Fut: 'r>(mut f: F) -> impl Stream<Item = A> + 'r
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<PaginatedApiResult<A>, ApiError>>,
    A: std::fmt::Debug,
{
    let initial_result = f(1);

    initial_result
        .map_ok(|paginated| {
            let PaginatedApiResult {
                total_pages, items, ..
            } = paginated;

            (total_pages, items)
        })
        .unwrap_or_else(|_| (0, Vec::default()))
        .map(|(total_pages, items)| {
            let initial_stream = stream::iter(items);
            let rest_stream = (2..=total_pages)
                .into_iter()
                .map(move |page| {
                    f(page)
                        .map_ok(|result| result.items)
                        .unwrap_or_else(|_| Vec::<A>::default())
                        .map(|items| stream::iter(items))
                })
                .collect::<FuturesUnordered<_>>()
                .flatten();

            initial_stream.chain(rest_stream)
        })
        .flatten_stream()
}

impl Api {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
        }
    }

    fn request_data<'a, 'r: 'a, 'b, A: 'r>(
        &'a self,
        payload: &'b Value,
    ) -> impl Future<Output = Result<PaginatedApiResult<A>, ApiError>> + 'r
    where
        A: DeserializeOwned + std::fmt::Debug,
    {
        self.client
            .post(&self.url)
            .json(payload)
            .send()
            .map_err(|err| ApiError::NetworkError(err))
            .and_then(|resp| {
                resp.json::<ApiPayload<A>>()
                    .map_err(|_| ApiError::InvalidResponse)
            })
            .map_ok(|payload| payload.result)
    }

    pub fn list_accounts<'a, 'r: 'a>(
        &'a self,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Account>, ApiError>> + 'r {
        self.request_data::<Account>(&json!({
            "method": "account_list",
            "params": {
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn stream_accounts<'a, 'r: 'a>(
        &'a self,
        page_size: usize,
    ) -> impl Stream<Item = Account> + 'r {
        let api_ref = self.clone();
        let f = move |page| {
            debug!("Fetching accounts in page {}", page);

            api_ref
                .list_accounts(page, page_size)
                .inspect_ok(move |_| {
                    debug!("Done fetching accounts for page {}", page);
                })
                .inspect_err(|err| {
                    debug!("Error fetching accounts: {}", err);
                })
        };

        stream_paginated(f)
    }

    pub fn list_claims_by_account_id<'a, 'b, 'r: 'a>(
        &'a self,
        account_id: &'b str,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Claim>, ApiError>> + 'r {
        self.request_data::<Claim>(&json!({
            "method": "claim_list",
            "params": {
                "account_id": account_id,
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn stream_claims_by_account_id<'a, 'r: 'a>(
        &'a self,
        account_id: String,
        page_size: usize,
    ) -> impl Stream<Item = Claim> + 'r {
        let api = self.clone();
        let f = move |page| {
            debug!("Fetching claims of account {} in page {}", account_id, page);

            let inner_account_id = account_id.clone();

            api.list_claims_by_account_id(&account_id, page, page_size)
                .inspect_ok(move |_| {
                    debug!(
                        "Done fetching claims for account {} in page {}",
                        inner_account_id, page
                    );
                })
                .inspect_err(|err| {
                    debug!("Error fetching claims: {}", err);
                })
        };

        stream_paginated(f)
    }

    pub fn list_comments_by_claim_id<'a, 'b, 'r: 'a>(
        &'a self,
        claim_id: &'b str,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Comment>, ApiError>> + 'r {
        self.request_data::<Comment>(&json!({
            "method": "comment_list",
            "params": {
                "claim_id": claim_id,
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn stream_comments_by_claim_id<'a, 'b, 'r: 'a>(
        &'a self,
        claim_id: String,
        page_size: usize,
    ) -> impl Stream<Item = Comment> + 'r {
        let api = self.clone();
        let f = move |page| {
            debug!("Fetching comment of claim {} in page {}", &claim_id, page);

            let inner_claim_id = claim_id.clone();

            api.list_comments_by_claim_id(&claim_id, page, page_size)
                .inspect_ok(move |_| {
                    debug!(
                        "Done fetching comments for claim {} in page {}",
                        inner_claim_id, page
                    );
                })
                .inspect_err(|err| {
                    debug!("Error fetching comments: {}", err);
                })
        };

        stream_paginated(f)
    }
}

pub struct Storage {
    conn: SqliteConnection,
}

impl Storage {
    pub fn open(database_url: String) -> Result<Self, diesel::ConnectionError> {
        SqliteConnection::establish(&database_url).map(|conn| {
            embedded_migrations::run(&conn).expect(&format!("Unable to run migrations"));

            Self { conn }
        })
    }

    pub fn save_comment(
        &self,
        account: Account,
        claim: Claim,
        comment: Comment,
    ) -> Result<CommentEntity, diesel::result::Error> {
        let Account { id: account_id, .. } = account;

        let Claim {
            name: claim_name, ..
        } = claim;

        let Comment {
            id,
            claim_id,
            commenter_id,
            commenter_name,
            commenter_url,
            comment,
            is_hidden,
            timestamp,
            ..
        } = comment;

        let new_comment = CommentEntity {
            id,
            account_id,
            claim_id,
            claim_name,
            commenter_id,
            commenter_name,
            commenter_url,
            comment,
            is_hidden,
            timestamp: timestamp.naive_utc(),
        };

        diesel::insert_into(comments::table)
            .values(&new_comment)
            .execute(&self.conn)
            .map(|_| new_comment)
    }

    pub fn get_comment_by_id(&self, comment_id: String) -> Option<CommentEntity> {
        use self::schema::comments::dsl::comments as c;

        c.find(comment_id).first(&self.conn).ok()
    }

    pub fn delete_comment_by_id(&self, comment_id: String) -> Result<(), diesel::result::Error> {
        use self::schema::comments::dsl::{comments as c, id};

        diesel::delete(c.filter(id.eq(comment_id)))
            .execute(&self.conn)
            .map(|_| ())
    }

    pub fn transaction<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce() -> Result<T, E>,
        E: From<diesel::result::Error>,
    {
        self.conn.transaction(f)
    }

    pub fn test_transaction<T, E, F>(&self, f: F) -> T
    where
        F: FnOnce() -> Result<T, E>,
        E: std::fmt::Debug,
    {
        self.conn.test_transaction(f)
    }
}

#[derive(Clone, Debug)]
pub struct Emails {
    from: String,
    to: String,
}

impl Emails {
    pub fn new(from: String, to: String) -> Self {
        Self { from, to }
    }

    pub fn notification_email(&self, comment: CommentEntity) -> Email {
        EmailBuilder::new()
            .to(self.to.to_string())
            .from(self.from.to_string())
            .subject(format!(
                "New Comment from {} on {}",
                comment.commenter_name, comment.claim_name
            ))
            .text(format!(
                "
      {}
      ---

      {} ({})
      {}
      ===
      {}
",
                comment.claim_name,
                comment.commenter_name,
                comment.commenter_url,
                comment.timestamp,
                comment.comment
            ))
            .build()
            .expect("Could not build email")
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use futures::stream::StreamExt;
    use rand::seq::SliceRandom;

    use crate::{Account, Api, Claim, Comment, Emails, Storage};

    const TEST_DB: &str = "test.db";
    const TEST_URL: &str = "http://localhost:5279";

    #[test]
    fn storage_should_work() {
        let storage = Storage::open(TEST_DB.to_string()).expect("Unable to connect");
        let emails = Emails::new("from@mail.com".to_string(), "to@mail.com".to_string());

        storage.test_transaction::<_, diesel::result::Error, _>(|| {
            let account = Account {
                id: "id".to_string(),
                name: "name".to_string(),
                is_default: true,
            };

            let claim = Claim {
                id: "id".to_string(),
                name: "name".to_string(),
                timestamp: Utc::now(),
            };

            let comment = Comment {
                id: "id".to_string(),
                claim_id: "claim_id".to_string(),
                comment: "comment".to_string(),
                commenter_id: "commenter_id".to_string(),
                commenter_name: "commenter_name".to_string(),
                commenter_url: "commenter_url".to_string(),
                is_hidden: false,
                timestamp: Utc::now(),
            };

            let saved_comment = storage
                .save_comment(account, claim, comment)
                .expect("Unable to save");

            let entity = storage
                .get_comment_by_id(saved_comment.id)
                .expect("Unable to fetch");

            dbg!(emails.notification_email(entity));

            Ok(())
        });
    }

    #[tokio::test]
    async fn api_stream_should_work() {
        let rng = &mut rand::thread_rng();
        let api = Api::new(TEST_URL.to_string());

        let account_ids = api
            .stream_accounts(100)
            .map(|account| account.id)
            .collect::<Vec<String>>()
            .await;

        let account_id = account_ids.choose(rng).unwrap_or(&"".to_string()).clone();
        if account_id.is_empty() {
            return;
        }

        let claim_ids = api
            .stream_claims_by_account_id(account_id, 100)
            .map(|claim| claim.id)
            .collect::<Vec<String>>()
            .await;

        let claim_id = claim_ids.choose(rng).unwrap_or(&"".to_string()).clone();
        if claim_id.is_empty() {
            return;
        }

        let comment_ids = api
            .stream_comments_by_claim_id(claim_id, 100)
            .map(|comment| comment.id)
            .collect::<Vec<String>>()
            .await;

        let _comment_id = comment_ids.choose(rng).unwrap_or(&"".to_string()).clone();
    }
}
