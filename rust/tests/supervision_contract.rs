//! Executable Rust *model* of the proposed Atto supervision contract.
//! This does NOT execute Atto source or prove runtime actor/lease behavior.
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Fenced,
    Recovering,
    Ready,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    Client,
    StorageChild,
    StorageSupervisor,
    Integrity,
    Root,
}

#[derive(Clone, Copy, Debug)]
struct Ticket {
    id: u64,
    epoch: u64,
}

#[derive(Clone, Copy)]
struct RecoveryProof {
    manifest_valid: bool,
    wal_crc_valid: bool,
    replay_complete: bool,
    wal_writer_bound: bool,
    restored_shards: usize,
    required_shards: usize,
}

impl RecoveryProof {
    fn valid(shards: usize) -> Self {
        Self {
            manifest_valid: true,
            wal_crc_valid: true,
            replay_complete: true,
            wal_writer_bound: true,
            restored_shards: shards,
            required_shards: shards,
        }
    }

    fn is_complete(self) -> bool {
        self.manifest_valid
            && self.wal_crc_valid
            && self.replay_complete
            && self.wal_writer_bound
            && self.required_shards > 0
            && self.restored_shards == self.required_shards
    }
}

struct SupervisionModel {
    epoch: u64,
    next_ticket: u64,
    phase: Phase,
    exclusive_store_lease: bool,
    permanently_blocked: bool,
    active: BTreeMap<u64, u64>,
    recent_restarts: VecDeque<u64>,
    max_restarts: usize,
    restart_window: u64,
}

impl SupervisionModel {
    fn new(max_restarts: usize, restart_window: u64) -> Self {
        Self {
            epoch: 0,
            next_ticket: 0,
            phase: Phase::Fenced,
            exclusive_store_lease: true,
            permanently_blocked: false,
            active: BTreeMap::new(),
            recent_restarts: VecDeque::new(),
            max_restarts,
            restart_window,
        }
    }

    fn begin_recovery(&mut self) -> Result<u64, &'static str> {
        if self.phase != Phase::Fenced || !self.active.is_empty() {
            return Err("old commands not drained or not fenced");
        }
        if !self.exclusive_store_lease || self.permanently_blocked {
            return Err("store lease or recovery permission unavailable");
        }
        let next = self.epoch.checked_add(1).ok_or("epoch exhausted")?;
        self.epoch = next;
        self.phase = Phase::Recovering;
        Ok(next)
    }

    fn publish_ready(&mut self, epoch: u64, proof: RecoveryProof) -> Result<(), &'static str> {
        if self.phase != Phase::Recovering || self.epoch != epoch {
            return Err("stale or out-of-order readiness");
        }
        if !self.exclusive_store_lease || !self.active.is_empty() || !proof.is_complete() {
            return Err("incomplete durable recovery proof");
        }
        self.phase = Phase::Ready;
        Ok(())
    }

    fn admit(&mut self, epoch: u64) -> Result<Ticket, &'static str> {
        if self.phase != Phase::Ready || epoch != self.epoch {
            return Err("storage fenced or stale routing reference");
        }
        let id = self.next_ticket.checked_add(1).ok_or("ticket id exhausted")?;
        self.next_ticket = id;
        self.active.insert(id, epoch);
        Ok(Ticket { id, epoch })
    }

    fn complete(&mut self, ticket: Ticket) -> Result<(), &'static str> {
        if self.active.get(&ticket.id) != Some(&ticket.epoch) {
            return Err("duplicate, forged or stale completion");
        }
        self.active.remove(&ticket.id);
        Ok(())
    }

    fn child_failure(
        &mut self,
        epoch: u64,
        failure: Failure,
        tick: u64,
    ) -> Result<(), &'static str> {
        if failure == Failure::Root {
            self.phase = Phase::Stopped;
            self.permanently_blocked = true;
            return Err("root failure requires process fail-stop");
        }
        if failure == Failure::Client {
            return Ok(()); // One-for-one network restart does not fence storage.
        }
        if epoch != self.epoch {
            return Ok(()); // Never fence a healthy new generation on stale CONTROL.
        }
        self.phase = Phase::Fenced; // Atomic admission close is the first step.
        if failure == Failure::Integrity {
            self.permanently_blocked = true;
            return Err("corrupt storage is fatal, do not restart-loop");
        }
        // A bounded window counts storage CHILD and SUPERVISOR crashes alike.
        self.recent_restarts
            .retain(|at| tick.saturating_sub(*at) < self.restart_window);
        if self.recent_restarts.len() >= self.max_restarts {
            self.permanently_blocked = true;
            return Err("restart intensity exceeded");
        }
        self.recent_restarts.push_back(tick);
        Ok(())
    }
}

#[test]
fn boot_requires_full_recovery_before_readiness() {
    let mut m = SupervisionModel::new(3, 100);
    assert!(m.admit(0).is_err());
    let epoch = m.begin_recovery().unwrap();
    let incomplete = RecoveryProof {
        wal_crc_valid: false,
        ..RecoveryProof::valid(4)
    };
    assert!(m.publish_ready(epoch, incomplete).is_err());
    assert_eq!(m.phase, Phase::Recovering);
    assert!(m.admit(epoch).is_err());
    m.publish_ready(epoch, RecoveryProof::valid(4)).unwrap();
    assert!(m.admit(epoch).is_ok());
}

#[test]
fn missing_shard_or_writer_blocks_readiness() {
    let mut m = SupervisionModel::new(3, 100);
    let epoch = m.begin_recovery().unwrap();
    let missing = RecoveryProof {
        restored_shards: 3,
        ..RecoveryProof::valid(4)
    };
    assert!(m.publish_ready(epoch, missing).is_err());
    let missing_writer = RecoveryProof {
        wal_writer_bound: false,
        ..RecoveryProof::valid(4)
    };
    assert!(m.publish_ready(epoch, missing_writer).is_err());
}

#[test]
fn fenced_generation_must_drain_before_recovery() {
    let mut m = SupervisionModel::new(3, 100);
    let old_epoch = m.begin_recovery().unwrap();
    m.publish_ready(old_epoch, RecoveryProof::valid(4)).unwrap();
    let accepted = m.admit(old_epoch).unwrap();
    m.child_failure(old_epoch, Failure::StorageChild, 10).unwrap();
    assert!(m.admit(old_epoch).is_err());
    assert!(m.begin_recovery().is_err());
    m.complete(accepted).unwrap();
    let new_epoch = m.begin_recovery().unwrap();
    assert_ne!(old_epoch, new_epoch);
    assert!(m.admit(old_epoch).is_err());
    m.publish_ready(new_epoch, RecoveryProof::valid(4)).unwrap();
    assert!(m.admit(new_epoch).is_ok());
}

#[test]
fn duplicate_and_forged_ticket_completions_fail_closed() {
    let mut m = SupervisionModel::new(3, 100);
    let epoch = m.begin_recovery().unwrap();
    m.publish_ready(epoch, RecoveryProof::valid(2)).unwrap();
    let ticket = m.admit(epoch).unwrap();
    assert!(m.complete(Ticket { id: ticket.id, epoch: epoch + 1 }).is_err());
    assert_eq!(m.active.len(), 1);
    m.complete(ticket).unwrap();
    assert!(m.complete(ticket).is_err());
}

#[test]
fn stale_supervisor_notice_cannot_fence_new_generation() {
    let mut m = SupervisionModel::new(3, 100);
    let old = m.begin_recovery().unwrap();
    m.publish_ready(old, RecoveryProof::valid(1)).unwrap();
    m.child_failure(old, Failure::StorageSupervisor, 1).unwrap();
    let new_epoch = m.begin_recovery().unwrap();
    m.publish_ready(new_epoch, RecoveryProof::valid(1)).unwrap();
    m.child_failure(old, Failure::StorageChild, 2).unwrap();
    assert_eq!(m.phase, Phase::Ready);
    assert!(m.admit(new_epoch).is_ok());
}

#[test]
fn client_crash_does_not_restart_storage() {
    let mut m = SupervisionModel::new(3, 100);
    let epoch = m.begin_recovery().unwrap();
    m.publish_ready(epoch, RecoveryProof::valid(1)).unwrap();
    m.child_failure(epoch, Failure::Client, 0).unwrap();
    assert_eq!(m.phase, Phase::Ready);
    assert_eq!(m.epoch, epoch);
    assert!(m.recent_restarts.is_empty());
}

#[test]
fn fatal_integrity_error_never_auto_restarts() {
    let mut m = SupervisionModel::new(3, 100);
    let epoch = m.begin_recovery().unwrap();
    m.publish_ready(epoch, RecoveryProof::valid(1)).unwrap();
    assert!(m.child_failure(epoch, Failure::Integrity, 1).is_err());
    assert_eq!(m.phase, Phase::Fenced);
    assert!(m.begin_recovery().is_err());
    assert!(m.admit(epoch).is_err());
}

#[test]
fn repeated_storage_failures_exhaust_restart_budget() {
    let mut m = SupervisionModel::new(2, 100);
    for tick in 0..2 {
        let epoch = m.begin_recovery().unwrap();
        m.publish_ready(epoch, RecoveryProof::valid(1)).unwrap();
        m.child_failure(epoch, Failure::StorageChild, tick).unwrap();
    }
    let epoch = m.begin_recovery().unwrap();
    m.publish_ready(epoch, RecoveryProof::valid(1)).unwrap();
    assert!(m.child_failure(epoch, Failure::StorageSupervisor, 2).is_err());
    assert_eq!(m.phase, Phase::Fenced);
    assert!(m.begin_recovery().is_err());
}

#[test]
fn restart_budget_resets_outside_window() {
    let mut m = SupervisionModel::new(1, 10);
    let first = m.begin_recovery().unwrap();
    m.publish_ready(first, RecoveryProof::valid(1)).unwrap();
    m.child_failure(first, Failure::StorageChild, 1).unwrap();
    let second = m.begin_recovery().unwrap();
    m.publish_ready(second, RecoveryProof::valid(1)).unwrap();
    m.child_failure(second, Failure::StorageChild, 12).unwrap();
    assert!(m.begin_recovery().is_ok());
}

#[test]
fn exclusive_process_store_lease_is_required() {
    let mut m = SupervisionModel::new(3, 100);
    m.exclusive_store_lease = false;
    assert!(m.begin_recovery().is_err());
    m.exclusive_store_lease = true;
    let epoch = m.begin_recovery().unwrap();
    m.exclusive_store_lease = false;
    assert!(m.publish_ready(epoch, RecoveryProof::valid(1)).is_err());
}

#[test]
fn epoch_overflow_cannot_reuse_generation() {
    let mut m = SupervisionModel::new(3, 100);
    m.epoch = u64::MAX;
    assert!(m.begin_recovery().is_err());
    assert_eq!(m.phase, Phase::Fenced);
}

#[test]
fn root_failure_requires_process_fail_stop() {
    let mut m = SupervisionModel::new(3, 100);
    let epoch = m.begin_recovery().unwrap();
    m.publish_ready(epoch, RecoveryProof::valid(1)).unwrap();
    assert!(m.child_failure(epoch, Failure::Root, 1).is_err());
    assert_eq!(m.phase, Phase::Stopped);
    assert!(m.begin_recovery().is_err());
}

#[test]
fn atto_supervision_contract_is_not_executable_evidence() {
    let source = include_str!("../../src/redis_supervision.atto");
    let kv = include_str!("../../src/kv_database.atto");
    assert!(source.contains("NATIVE_COMPILE=NOT_EXECUTED"));
    for name in [
        "RedisRootSupervisor",
        "StorageSupervisor",
        "NetworkSupervisor",
        "MaintenanceSupervisor",
        "StorageAdmission.fence_and_drain",
        "StorageAdmission.publish_ready",
        "StorageRecovery.load_and_verify",
        "fail-stop",
    ] {
        assert!(source.contains(name), "missing supervision requirement: {name}");
    }
    // Source-shape conformance only: no compiler/parser/runtime is exercised.
    // A supervisor is a distinct nominal type and must not inherit Actor.
    for declaration in [
        "define pub supervisor RedisRootSupervisor as",
        "define supervisor StorageSupervisor as",
        "define supervisor NetworkSupervisor as",
        "define supervisor MaintenanceSupervisor as",
    ] {
        assert!(source.contains(declaration), "missing nominal supervisor: {declaration}");
    }
    assert!(!source.contains("extends Supervisor"));
    assert_eq!(source.matches("self.spawn_supervisor(").count(), 4);
    assert!(source.contains("self.spawn_actor(WalWriter(), restart=#permanent)"));
    assert!(kv.contains("final u64 storage_epoch;"));
    assert_eq!(kv.matches("StorageAdmission.enter(self.storage_epoch)").count(), 3);
    // The public router is not a sufficient guard: direct WAL/shard calls
    // and late spill installation also require a live matching epoch.
    assert_eq!(kv.matches("StorageAdmission.enter(storage_epoch)").count(), 7);
    assert_eq!(kv.matches("StorageAdmission.require_actor_generation").count(), 5);
    assert_eq!(kv.matches("WalStorage.require_current_writer").count(), 2);
    assert_eq!(kv.matches("wal.append_owned(storage_epoch").count(), 2);
    let checkpoint = include_str!("../../src/checkpoint.atto");
    assert!(checkpoint.contains("StorageAdmission.enter(storage_epoch)"));
    assert!(checkpoint.contains("storage_epoch, rt take segment, generation_id"));
}
