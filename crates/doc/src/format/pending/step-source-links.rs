//! STEP inserts may keep their source path, digest and naming policy in a component link, and an
//! import made by a plugin importer may keep its source path, digest and the importer (the
//! plugin's id, version and entry, and the options it was given). Older designs have no such
//! links, and a STEP link with no importer is a built-in STEP read; they retain their independent
//! imported bodies. The last release asks for an update instead of opening a design carrying this
//! format change.
