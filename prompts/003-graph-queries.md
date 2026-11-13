# 003: Graph queries in S3A

Goal: the graph becomes queryable shared memory, not just stored edges.
1. Add a generic edge record to core (from, to, edge_type, properties, signer, timestamp) on a new core tile type; Scholar Tile 13 edges map onto it via the pack, without breaking existing Tile 13 files.
2. Add QL support: `TRAVERSE FROM <node> VIA <edge_type> DEPTH <n>`, `NEIGHBORS <node>`, `PATH <a> TO <b>`. Back it with an adjacency index rebuilt on open.
3. CLI: `s3a graph neighbors|path|traverse`.
4. Use cases to test: assembly tree (part -> child part), citation graph (paper -> cites -> paper), agent grants (agent -> acts_for -> person).
Tests for each use case, plus a 100k-edge performance test. Clippy, workspace tests, log, commit, push.
