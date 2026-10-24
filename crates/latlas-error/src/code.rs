// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-23: The registry itself: every [`ErrorCode`], its stable number and its form,
//! grouped by range block.
//!
//! Owner: latlas-error.
//! Invariants:
//! - Append-only: a code is never renumbered or removed; `codes.lock` and its test enforce that.
//! - The `Latlas` block keeps the low byte of the Metrale `KnowledgeGraph 0x0800` code each one
//!   was cut from (2026-10-23 K0), so the Metrale edge maps `0x01nn <-> 0x08nn` one to one.

use crate::registry::error_registry;

error_registry! {
    /// 2026-10-23: `0x0000–0x00FF`: universal codes any crate may use.
    General = 0x0000..=0x00FF {
        /// 2026-10-23: Unclassified; the one argument is the whole message.
        Generic = 0x0000 => "{}",
        /// 2026-10-23: An i/o failure a host reports; the argument describes it.
        Io = 0x0001 => "i/o error: {}",
        /// 2026-10-23: A caller passed a value the callee refuses: (argument, reason).
        InvalidArgument = 0x0002 => "invalid argument {}: {}",
        /// 2026-10-23: A declared, reported gap: (operation, backend or host).
        NotImplemented = 0x0003 => "{} is not yet implemented on {}",
        /// 2026-10-23: A resource or capability that is absent here: (what).
        Unavailable = 0x0004 => "{} is unavailable",
    }
    /// 2026-10-23: `0x0100–0x01FF`: Latlas: bundle integrity `0x0100–0x010F`, sources and
    /// records `0x0110–0x011F`, embedder `0x0120–0x012F`, store and tools `0x0130–0x013F`.
    Latlas = 0x0100..=0x01FF {
        /// 2026-10-23: A record names a node no source defines: (where, the reference).
        LatlasDanglingRef = 0x0100 => "{} references {}, which no source defines",
        /// 2026-10-23: Two records derive the same id: (id, first source, second source).
        LatlasDuplicateId = 0x0101 => "id {} is derived twice ({} and {})",
        /// 2026-10-23: Two typed ids hash to one u64 point id: (id, id, point id).
        LatlasIdCollision = 0x0102 => "ids {} and {} collide on point id {}",
        /// 2026-10-23: A ledger row its checker refuses: (where, reason).
        LatlasBadLedgerRow = 0x0103 => "{}: {}",
        /// 2026-10-23: A record that is not in its file's row format: (where, reason).
        LatlasBadRow = 0x0104 => "{}: {}",
        /// 2026-10-23: More node labels than a collection holds: (needed, limit).
        LatlasTooManyLabels = 0x0105 => "the ontology needs {} labels; a collection holds {}",
        /// 2026-10-23: A field matched the secrets denylist and was dropped: (where, pattern).
        LatlasSecretRedacted = 0x0106 => "{}: a field matched the secrets denylist ({}) and was dropped",
        /// 2026-10-23: A `gate` claim outside the grammar: (where, claim, reason).
        LatlasBadClaim = 0x0107 => "{}: claim `{}`: {}",
        /// 2026-10-23: An id that does not parse as `<label-prefix>:<key>`: (id, reason).
        LatlasBadId = 0x0108 => "id `{}`: {}",
        /// 2026-10-23: A memory `[[wikilink]]` to a memory not written yet: a deliberate forward
        /// reference (the memory rules), reported apart from integrity errors: (at, target).
        LatlasForwardLink = 0x0109 => "{} links to {}, which is not written yet",
        /// 2026-10-23: The LKB's own inventory script refused to build (its model is strict):
        /// (script, its first refusal line and count). The graph keeps the rest of the LKB.
        LatlasInventoryRefused = 0x010A => "{} refused to build the kernel inventory: {}",
        /// 2026-10-23: The inventory script built the inventory but reported a problem other than
        /// an unclassified kernel: (script, `kind: text`). Not fatal; the inventory is kept.
        LatlasInventoryProblem = 0x010B => "{} reports an inventory problem: {}",
        /// 2026-10-23: A whole LKB file its own reader refuses (fatal; every node of it would be
        /// missing): (path, the reader's error).
        LatlasLkbFileRejected = 0x010C => "{} is refused as a whole by its reader: {}",
        /// 2026-10-23: A config field missing or invalid; there is no default: (field, reason).
        LatlasConfig = 0x0110 => "config {}: {}",
        /// 2026-10-23: A source its host could not read or stat: (path, reason).
        LatlasSource = 0x0111 => "source {}: {}",
        /// 2026-10-23: A file's own validator refused a record; the file is unchanged:
        /// (validator, its output).
        LatlasValidatorRejected = 0x0112 => "validator {} refused the record: {}",
        /// 2026-10-23: A record write that could not complete: (path, reason).
        LatlasRecordWrite = 0x0113 => "record {}: {}",
        /// 2026-10-23: The bundle could not be re-derived; results are stale: (reason).
        LatlasStaleBundle = 0x0114 => "the bundle is stale: {}",
        /// 2026-10-23: The embedder is in its declared unavailable mode or failed: (reason).
        LatlasEmbedderUnavailable = 0x0120 => "embedder unavailable: {}",
        /// 2026-10-23: The embedder kept rate-limiting: (attempts).
        LatlasEmbedRateLimited = 0x0121 => "embedder still rate-limited or failing (HTTP 429/5xx) after {} attempts",
        /// 2026-10-23: An embedder response outside its protocol: (reason).
        LatlasEmbedProtocol = 0x0122 => "embedder response: {}",
        /// 2026-10-23: An embedding of another width than the config declares: (got, declared).
        LatlasEmbedDim = 0x0123 => "embedder returned dimension {}; the config declares {}",
        /// 2026-10-23: The provider refused a request it should accept by what we sent (e.g. an
        /// input-length refusal when every input is within `max_input_chars`): (what it said and
        /// the request shape).
        LatlasEmbedderFault = 0x0124 => "embedder fault: {}",
        /// 2026-10-23: A Cypher text `la_query` refuses: (reason).
        LatlasCypherRejected = 0x0130 => "cypher refused: {}",
        /// 2026-10-23: A graph store operation failed: (operation, reason).
        LatlasStore = 0x0131 => "store {}: {}",
        /// 2026-10-23: A tool argument outside its schema: (argument, reason).
        LatlasToolArgument = 0x0132 => "tool argument {}: {}",
        /// 2026-10-23: An id that names no node: (id).
        LatlasUnknownId = 0x0133 => "no node {}",
    }
}
