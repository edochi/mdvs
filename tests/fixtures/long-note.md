---
title: "CRDT Conflict Resolution"
tags: [distributed-systems, crdt, papers]
date: 2025-03-20
category: research
author: edoardo
---

# CRDT Conflict Resolution

Conflict-free Replicated Data Types (CRDTs) are data structures that can be replicated across multiple computers in a network, where replicas can be updated independently and concurrently without coordination, and it is always mathematically possible to resolve inconsistencies.

## Background

Traditional distributed systems rely on consensus protocols like Paxos or Raft to maintain consistency. These protocols require coordination between nodes, which introduces latency and reduces availability during network partitions. CRDTs take a fundamentally different approach by designing data structures whose operations are commutative, associative, and idempotent.

The CAP theorem tells us we cannot have consistency, availability, and partition tolerance simultaneously. CRDTs choose availability and partition tolerance, achieving eventual consistency through mathematical properties rather than coordination.

## State-Based CRDTs (CvRDTs)

State-based CRDTs, also known as convergent replicated data types, propagate updates by shipping the entire state between replicas. The merge function must be commutative, associative, and idempotent — forming a join-semilattice.

### G-Counter (Grow-Only Counter)

A G-Counter is one of the simplest CRDTs. Each node maintains its own counter, and the state is a vector of all node counters. The merge operation takes the element-wise maximum.

```rust
struct GCounter {
    counts: HashMap<NodeId, u64>,
}

impl GCounter {
    fn increment(&mut self, node: NodeId) {
        *self.counts.entry(node).or_insert(0) += 1;
    }

    fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    fn merge(&mut self, other: &GCounter) {
        for (node, &count) in &other.counts {
            let entry = self.counts.entry(*node).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
}
```

### PN-Counter (Positive-Negative Counter)

A PN-Counter supports both increment and decrement by maintaining two G-Counters: one for increments and one for decrements. The value is the difference between the two.

## Operation-Based CRDTs (CmRDTs)

Operation-based CRDTs, or commutative replicated data types, propagate updates by transmitting only the operations. This requires a reliable broadcast channel that delivers operations exactly once. The operations must be commutative — the order of delivery doesn't matter.

### LWW-Register (Last-Writer-Wins Register)

The LWW-Register resolves conflicts by attaching a timestamp to each write. The write with the highest timestamp wins. This is simple but can lose updates silently.

### MV-Register (Multi-Value Register)

The MV-Register preserves all concurrent writes, letting the application resolve conflicts. This is what Amazon's Dynamo and Riak use — the "siblings" concept.

## Practical Considerations

CRDTs are not a silver bullet. They work well for certain data types (counters, sets, registers) but become complex for richer structures like text documents or relational data.

For collaborative text editing, approaches like RGA (Replicated Growable Array) or Yjs's YATA algorithm provide character-level CRDTs that maintain document structure across concurrent edits.

### Performance

The main concern with state-based CRDTs is the size of the state that needs to be shipped between replicas. Delta-state CRDTs address this by shipping only the changes since the last synchronization.

### Garbage Collection

Tombstones in CRDTs (markers for deleted elements) can accumulate over time. Garbage collection requires coordination, which partially defeats the purpose of coordination-free operation. Various approaches exist, from epoch-based collection to causal stability.
