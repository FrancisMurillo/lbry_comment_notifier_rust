table! {
    comments (id) {
        id -> Text,
        account_id -> Text,
        claim_id -> Text,
        claim_name -> Text,
        commenter_id -> Text,
        commenter_name -> Text,
        commenter_url -> Text,
        comment -> Text,
        is_hidden -> Bool,
        timestamp -> Timestamp,
    }
}
