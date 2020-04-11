use chrono::{DateTime, Utc};
use futures::{
    future::{ok, TryFutureExt},
    prelude::Future,
    stream::{FuturesUnordered, Stream},
};
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, value::Value};

#[derive(Debug, Deserialize)]
struct Account {
    id: String,
    name: String,
    is_default: bool,
}

#[derive(Debug, Deserialize)]
struct Claim {
    #[serde(rename(deserialize = "claim_id"))]
    id: String,
    name: String,
    #[serde(with = "date_format")]
    timestamp: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    #[serde(rename(deserialize = "comment_id"))]
    id: String,
    comment: String,
    #[serde(rename(deserialize = "channel_id"))]
    commenter_id: String,
    #[serde(rename(deserialize = "channel_name"))]
    commenter_name: String,
    #[serde(rename(deserialize = "channel_url"))]
    commenter_url: String,

    is_hidden: bool,

    #[serde(with = "date_format")]
    timestamp: DateTime<Utc>,
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

struct Api {
    client: Client,
}

#[derive(Debug)]
enum ApiError {
    InvalidResponse,
    NetworkError(reqwest::Error),
}

#[derive(Debug, Deserialize)]
struct ApiPayload<A> {
    result: PaginatedApiResult<A>,
}

#[derive(Debug, Deserialize)]
struct PaginatedApiResult<A> {
    items: Vec<A>,
    page: usize,
    page_size: usize,
    total_items: usize,
    total_pages: usize,
}

struct Cursor {
    client: Client,
}

impl Api {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    fn request_data<A>(
        &self,
        payload: &Value,
    ) -> impl Future<Output = Result<PaginatedApiResult<A>, ApiError>>
    where
        A: DeserializeOwned,
    {
        self.client
            .post("http://pi:5279")
            .json(payload)
            .send()
            .map_err(|err| ApiError::NetworkError(err))
            .and_then(|resp| {
                resp.json::<ApiPayload<A>>()
                    .map_err(|_| ApiError::InvalidResponse)
            })
            .map_ok(|payload| payload.result)
    }

    pub fn list_accounts(
        &self,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Account>, ApiError>> {
        self.request_data::<Account>(&json!({
            "method": "account_list",
            "params": {
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn list_claims_by_account_id(
        &self,
        account_id: &str,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Claim>, ApiError>> {
        self.request_data::<Claim>(&json!({
            "method": "claim_list",
            "params": {
                "account_id": account_id,
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn list_comments_by_claim_id(
        &self,
        claim_id: &str,
        page: usize,
        page_size: usize,
    ) -> impl Future<Output = Result<PaginatedApiResult<Comment>, ApiError>> {
        self.request_data::<Comment>(&json!({
            "method": "comment_list",
            "params": {
                "claim_id": claim_id,
                "page": page,
                "page_size": page_size,
            }
        }))
    }

    pub fn consume_paginated<
        A,
        F: Fn(usize, usize) -> dyn Future<Output = Result<PaginatedApiResult<A>, ApiError>>,
    >(
        f: F,
        page_size: usize,
    ) -> impl Future<Output = Result<Vec<A>, ApiError>> {
        f(1, page_size).map_ok(|paginated| {
            let PaginatedApiResult {
                items: items,
                total_pages: total_pages,
                ..
            } = paginated;

            if total_pages > 1 {
                let collection = FuturesUnordered::new();
                collection.push(ok(Ok(items)));

                (2..total_pages).into_iter().for_each(|this_page| {
                    let next_fut = f(this_page, page_size).map_ok(|next_result| {
                        next_result.map_ok(|next_paginated| next_paginated.items)
                    });
                    collection.push();
                });

                collection
            } else {
                Ok(items)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::Api;

    #[tokio::test]
    async fn it_works_still() {
        let api = Api::new();
        dbg!(api.list_accounts(0, 0).await);
    }

    #[tokio::test]
    async fn it_works() {
        let api = Api::new();
        dbg!(api.list_accounts(1, 10).await);
        dbg!(
            api.list_claims_by_account_id(&"bZN1bBvkhi68t4SvHHZXztH8r1hRSUbjo7", 1, 10)
                .await
        );
        dbg!(
            api.list_comments_by_claim_id(&"97aeed54f3712a32eed19bfb40ca4e6000b9d2fa", 1, 10)
                .await
        );

        assert_eq!(2 + 2, 4);
    }
}
