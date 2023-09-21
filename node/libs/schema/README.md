# Schema

This crate contains all the protobuf schemas used by the node's crates.
The schemas are located in the `proto` directory.
The layout of the `proto` directory is the same as the layout of the node's
workspace: for example the schemas used by the `types` crate reside in
`schema/proto/types` directory.

`build.rs` generates rust code from the schemas.
Again, the module layout of the generated code is the same as the layout of
the node's workspace: for example the code generated from schemas used by the `types` crate
reside in `schema::types` module.

Each crate owning the given part of the schema is expected to export an alias to
the generated code, for example `types::validator::schema` is an alias for `schema::types::validator`.
They are also expected to implement a strongly-typed rust representations manually and conversion to
the generated code by implementing `schema::ProtoFmt` trait for them.

If crate A depends on crate B, crate A is expected to use the strongly-typed representations defined in crate B,
and use the schema-generated types of crate B only to define conversion to schema-generated types of crate A.
For example, crate `network` has a type `network::Message` which contains a field of type `types::validator::PublicKey`.
To define a conversion from `network::Message` to `network::schema::Message`, a conversion
from `types::validator::PublicKey` to `types::validator::schema::PublicKey` is used.

## Why not place each schema directly to the corresponding crate?

Generating rust code from schemas scattered over many crates is hard,
because of how cargo works, and how the code-generating library has been
implemented.
