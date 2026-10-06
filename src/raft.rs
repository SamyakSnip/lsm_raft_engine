use std::collections::HashMap;
use crate::coding::{decode_bytes, encode_bytes};
use crate::engine::LsmEngine;
use crate::error::Result;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub term: u64,
    pub index: u64,
    pub command: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RequestVoteArgs {
    pub term: u64,
    pub candidate_id: u64,
    pub last_log_index: u64,
    pub last_log_term: u64,
}

#[derive(Debug, Clone)]
pub struct RequestVoteReply {
    pub term: u64,
    pub vote_granted: bool,
}

#[derive(Debug, Clone)]
pub struct AppendEntriesArgs {
    pub term: u64,
    pub leader_id: u64,
    pub prev_log_index: u64,
    pub prev_log_term: u64,
    pub entries: Vec<LogEntry>,
    pub leader_commit: u64,
}

#[derive(Debug, Clone)]
pub struct AppendEntriesReply {
    pub term: u64,
    pub success: bool,
}

pub struct RaftNode {
    pub id: u64,
    pub peers: Vec<u64>,
    pub current_term: u64,
    pub voted_for: Option<u64>,
    pub log: Vec<LogEntry>,
    pub role: Role,
    pub commit_index: u64,
    pub last_applied: u64,
    pub votes_received: usize,
    pub match_indices: HashMap<u64, u64>, // Tracks how much of the log each peer has replicated
}

impl RaftNode {
        pub fn new(id: u64, peers: Vec<u64>) -> Self {
        Self {
            id,
            peers,
            current_term: 0,
            voted_for: None,
            log: Vec::new(),
            role: Role::Follower,
            commit_index: 0,
            last_applied: 0,
            votes_received: 0,
            match_indices: HashMap::new(), // <--- Add this line!
        }
    }


    /// Transitions to Candidate and starts an election.
    pub fn start_election(&mut self) -> RequestVoteArgs {
        self.role = Role::Candidate;
        self.current_term += 1;
        self.voted_for = Some(self.id);
        self.votes_received = 1; // Votes for itself!

        let last_log_term = self.log.last().map_or(0, |e| e.term);
        let last_log_index = self.log.len() as u64;

        RequestVoteArgs {
            term: self.current_term,
            candidate_id: self.id,
            last_log_index,
            last_log_term,
        }
    }

    /// Handles an incoming RequestVote RPC.
    pub fn handle_request_vote(&mut self, args: &RequestVoteArgs) -> RequestVoteReply {
        // 1. If candidate's term is older, reject vote!
        if args.term < self.current_term {
            return RequestVoteReply {
                term: self.current_term,
                vote_granted: false,
            };
        }

        // 2. If candidate's term is newer, step down to Follower!
        if args.term > self.current_term {
            self.current_term = args.term;
            self.role = Role::Follower;
            self.voted_for = None;
        }

        // 3. Check if we haven't voted, or already voted for this candidate
        let can_vote = self.voted_for.is_none() || self.voted_for == Some(args.candidate_id);

        // 4. Log up-to-date check (Election Safety)
        let my_last_term = self.log.last().map_or(0, |e| e.term);
        let my_last_index = self.log.len() as u64;
        let log_ok = args.last_log_term > my_last_term
            || (args.last_log_term == my_last_term && args.last_log_index >= my_last_index);

        if can_vote && log_ok {
            self.voted_for = Some(args.candidate_id);
            RequestVoteReply {
                term: self.current_term,
                vote_granted: true,
            }
        } else {
            RequestVoteReply {
                term: self.current_term,
                vote_granted: false,
            }
        }
    }

            /// Client proposes a key-value write to the Leader.
    /// Encodes (key, value) into a log command and generates AppendEntries for peers.
    pub fn propose(&mut self, key: &[u8], value: &[u8]) -> AppendEntriesArgs {
        let mut command = Vec::new();
        encode_bytes(key, &mut command);
        encode_bytes(value, &mut command);

        let prev_log_term = self.log.last().map_or(0, |e| e.term);
        let prev_log_index = self.log.len() as u64;

        let index = prev_log_index + 1;
        let entry = LogEntry {
            term: self.current_term,
            index,
            command,
        };
        self.log.push(entry.clone());

        AppendEntriesArgs {
            term: self.current_term,
            leader_id: self.id,
            prev_log_index,
            prev_log_term,
            entries: vec![entry],
            leader_commit: self.commit_index,
        }
    }

    /// Leader records an acknowledgment from a peer.
    /// If an entry is replicated on a Majority, it becomes COMMITTED!
    pub fn record_ack(&mut self, peer_id: u64, matched_index: u64) -> bool {
        self.match_indices.insert(peer_id, matched_index);

        // Check if matched_index has reached a majority
        // (Leader itself already has the entry, so count starts at 1)
        let mut acks = 1;
        for &idx in self.match_indices.values() {
            if idx >= matched_index {
                acks += 1;
            }
        }

        let majority = (self.peers.len() + 1) / 2 + 1;
        if acks >= majority && matched_index > self.commit_index {
            self.commit_index = matched_index;
            true // Newly committed!
        } else {
            false
        }
    }

    /// Applies all committed, unapplied log entries to the local LsmEngine state machine.
    pub fn apply_committed(&mut self, engine: &LsmEngine) -> Result<usize> {
        let mut applied_count = 0;

        while self.last_applied < self.commit_index {
            let entry_idx = self.last_applied as usize;
            if entry_idx >= self.log.len() {
                break;
            }

            let entry = &self.log[entry_idx];
            // Decode (key, value) from the log command
            let (key, rest) = decode_bytes(&entry.command)?;
            let (val, _) = decode_bytes(rest)?;

            // Apply to the deterministic local LSM-tree engine!
            engine.put(key, val)?;

            self.last_applied += 1;
            applied_count += 1;
        }

        Ok(applied_count)
    }


    /// Handles an incoming AppendEntries RPC (Heartbeat & Log Replication).
    pub fn handle_append_entries(&mut self, args: &AppendEntriesArgs) -> AppendEntriesReply {
        // 1. If leader's term is older, reject!
        if args.term < self.current_term {
            return AppendEntriesReply {
                term: self.current_term,
                success: false,
            };
        }

        // 2. If term is newer or equal, acknowledge leader and step down to Follower
        if args.term >= self.current_term {
            self.current_term = args.term;
            self.role = Role::Follower;
            self.voted_for = None;
        }

        // 3. Heartbeat or log entries accepted!
        for entry in &args.entries {
            self.log.push(entry.clone());
        }

        AppendEntriesReply {
            term: self.current_term,
            success: true,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raft_election_and_voting() {
        // Create 3 nodes
        let mut node1 = RaftNode::new(1, vec![2, 3]);
        let mut node2 = RaftNode::new(2, vec![1, 3]);
        let mut node3 = RaftNode::new(3, vec![1, 2]);

        // 1. Node 1 times out and starts an election
        let vote_request = node1.start_election();
        assert_eq!(node1.role, Role::Candidate);
        assert_eq!(node1.current_term, 1);
        assert_eq!(node1.votes_received, 1);

        // 2. Node 2 and Node 3 receive RequestVote from Node 1
        let reply2 = node2.handle_request_vote(&vote_request);
        let reply3 = node3.handle_request_vote(&vote_request);

        assert!(reply2.vote_granted);
        assert!(reply3.vote_granted);

        // 3. If Node 1 receives vote from Node 2, it reaches majority (2 out of 3)!
        node1.votes_received += 1;
        if node1.votes_received >= 2 {
            node1.role = Role::Leader;
        }
        assert_eq!(node1.role, Role::Leader);

        // 4. Stale election: A node in term 0 cannot request votes from term 1 nodes
        let stale_request = RequestVoteArgs {
            term: 0,
            candidate_id: 3,
            last_log_index: 0,
            last_log_term: 0,
        };
        let reply = node1.handle_request_vote(&stale_request);
        assert!(!reply.vote_granted); // Stale candidate rejected!
    }

    #[test]
    fn test_raft_cluster_partition_and_recovery() {
        let dir1 = "test_raft_node1";
        let dir2 = "test_raft_node2";
        let dir3 = "test_raft_node3";
        let _ = std::fs::remove_dir_all(dir1);
        let _ = std::fs::remove_dir_all(dir2);
        let _ = std::fs::remove_dir_all(dir3);

        let engine1 = LsmEngine::open(dir1, 1024).unwrap();
        let engine2 = LsmEngine::open(dir2, 1024).unwrap();
        let engine3 = LsmEngine::open(dir3, 1024).unwrap();

        let mut node1 = RaftNode::new(1, vec![2, 3]);
        let mut node2 = RaftNode::new(2, vec![1, 3]);
        let mut node3 = RaftNode::new(3, vec![1, 2]);

        // 1. Elect Node 1 as Leader
        let vote_req = node1.start_election();
        node2.handle_request_vote(&vote_req);
        node1.record_ack(2, 0); // Node 2 votes for Node 1
        node1.role = Role::Leader;

        // 2. Client sends write: put("cluster_name", "lsm_raft_v1") to Leader (Node 1)
        let append_args = node1.propose(b"cluster_name", b"lsm_raft_v1");

        // 3. Node 2 receives AppendEntries and acknowledges
        let reply2 = node2.handle_append_entries(&append_args);
        assert!(reply2.success);

        // --- PARTITION EVENT ---
        // Node 3 is partitioned and does NOT receive this message!

        // 4. Leader records Node 2's acknowledgment. Quorum reached (2/3)!
        let newly_committed = node1.record_ack(2, 1);
        assert!(newly_committed);
        assert_eq!(node1.commit_index, 1);

        // 5. Apply on Leader and Follower 2
        node1.apply_committed(&engine1).unwrap();
        node2.commit_index = 1; // Follower learns commit index on next heartbeat
        node2.apply_committed(&engine2).unwrap();

        // Both Node 1 and Node 2 possess the committed data!
        assert_eq!(engine1.get(b"cluster_name").unwrap(), Some(b"lsm_raft_v1".to_vec()));
        assert_eq!(engine2.get(b"cluster_name").unwrap(), Some(b"lsm_raft_v1".to_vec()));
        // Node 3 is still empty because it was partitioned
        assert_eq!(engine3.get(b"cluster_name").unwrap(), None);

        // --- HEALING EVENT ---
        // Network partition heals! Leader synchronizes Node 3:
        let reply3 = node3.handle_append_entries(&append_args);
        assert!(reply3.success);
        node3.commit_index = 1;
        node3.apply_committed(&engine3).unwrap();

        // Node 3 has fully caught up!
        assert_eq!(engine3.get(b"cluster_name").unwrap(), Some(b"lsm_raft_v1".to_vec()));

        let _ = std::fs::remove_dir_all(dir1);
        let _ = std::fs::remove_dir_all(dir2);
        let _ = std::fs::remove_dir_all(dir3);
    }

}
