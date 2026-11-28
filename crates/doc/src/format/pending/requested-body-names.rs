//! Features may save optional `requested_body_names`, indexed like `body_names`: a nonempty
//! slot records the request that the pin pass replaced with a numbered fallback. Proven
//! configuration alternatives can restore that request; explicit names have no such record.
//! Older designs default to an empty list, so no prior name is inferred from spelling.
//! Older readers ignore the optional field. The last release warns about the newer format,
//! opens what it understands and preserves the unread field when saving.
