//! Tenant admission for the experimental native Cranelift host.
//! This is NOT machine-code sandboxing; untrusted native JIT guests require
//! a separately supervised OS process and an authenticated IPC boundary.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Phase {
    Created,
    Running,
    Quiescing,
    Stopped,
}

#[derive(Clone)]
pub struct IsolateHandle {
    tenant: String,
    generation: u64,
    authority: Arc<()>,
}
#[derive(Clone)]
pub struct ActorTicket {
    isolate: IsolateHandle,
    id: u64,
    epoch: u64,
    authority: Arc<()>,
}
pub struct ResourcePin {
    isolate: IsolateHandle,
    actor: ActorTicket,
    id: u64,
    authority: Arc<()>,
}
pub struct Snapshot {
    pub phase: Phase,
    pub generation: u64,
    pub actors: usize,
    pub active_pins: usize,
    pub active_guest_calls: usize,
    pub scalar_state: i64,
}
struct ActorState {
    epoch: u64,
    authority: Arc<()>,
    pins: usize,
}
struct PinState {
    actor_id: u64,
    actor_epoch: u64,
    authority: Arc<()>,
}
struct Tenant {
    phase: Phase,
    generation: u64,
    authority: Arc<()>,
    max_actors: usize,
    max_pins: usize,
    next_actor_epoch: u64,
    next_pin: u64,
    actors: HashMap<u64, ActorState>,
    pins: HashMap<u64, PinState>,
    scalar_state: i64,
    active_guest_calls: usize,
    guest_turn: Arc<Mutex<()>>,
}
struct Inner {
    max_tenants: usize,
    next_generation: u64,
    tenants: HashMap<String, Tenant>,
}
impl Inner {
    fn check(&self, h: &IsolateHandle) -> Result<&Tenant, &'static str> {
        let s = self.tenants.get(&h.tenant).ok_or("stale isolate")?;
        if s.generation != h.generation || !Arc::ptr_eq(&s.authority, &h.authority) {
            return Err("foreign or stale isolate");
        }
        Ok(s)
    }
    fn check_mut(&mut self, h: &IsolateHandle) -> Result<&mut Tenant, &'static str> {
        let s = self.tenants.get_mut(&h.tenant).ok_or("stale isolate")?;
        if s.generation != h.generation || !Arc::ptr_eq(&s.authority, &h.authority) {
            return Err("foreign or stale isolate");
        }
        Ok(s)
    }
}
fn check_actor<'a>(s: &'a Tenant, h: &ActorTicket) -> Result<&'a ActorState, &'static str> {
    if s.generation != h.isolate.generation || !Arc::ptr_eq(&s.authority, &h.isolate.authority) {
        return Err("foreign actor");
    }
    let actor = s.actors.get(&h.id).ok_or("stale actor")?;
    if actor.epoch != h.epoch || !Arc::ptr_eq(&actor.authority, &h.authority) {
        return Err("stale actor incarnation");
    }
    Ok(actor)
}
pub struct OresVM {
    inner: Mutex<Inner>,
}
impl OresVM {
    pub fn new(max_tenants: usize) -> Result<Self, &'static str> {
        if max_tenants == 0 || max_tenants > 1024 {
            return Err("tenant budget");
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                max_tenants,
                next_generation: 1,
                tenants: HashMap::new(),
            }),
        })
    }
    pub fn create_isolate(
        &self,
        tenant: &str,
        max_actors: usize,
        max_pins: usize,
    ) -> Result<IsolateHandle, &'static str> {
        if tenant.is_empty()
            || tenant.len() > 128
            || !tenant
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            || max_actors == 0
            || max_actors > 65536
            || max_pins == 0
            || max_pins > 65536
        {
            return Err("invalid tenant config");
        }
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        if g.tenants.contains_key(tenant) {
            return Err("duplicate tenant");
        }
        if g.tenants.len() >= g.max_tenants {
            return Err("tenant limit");
        }
        let generation = g.next_generation;
        g.next_generation = g
            .next_generation
            .checked_add(1)
            .ok_or("generation exhausted")?;
        let authority = Arc::new(());
        g.tenants.insert(
            tenant.to_owned(),
            Tenant {
                phase: Phase::Created,
                generation,
                authority: authority.clone(),
                max_actors,
                max_pins,
                next_actor_epoch: 1,
                next_pin: 1,
                actors: HashMap::new(),
                pins: HashMap::new(),
                scalar_state: 0,
                active_guest_calls: 0,
                guest_turn: Arc::new(Mutex::new(())),
            },
        );
        Ok(IsolateHandle {
            tenant: tenant.to_owned(),
            generation,
            authority,
        })
    }
    pub fn start(&self, h: &IsolateHandle) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        if s.phase != Phase::Created {
            return Err("invalid start phase");
        }
        s.phase = Phase::Running;
        Ok(())
    }
    pub fn register_actor(&self, h: &IsolateHandle, id: u64) -> Result<ActorTicket, &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        if s.phase != Phase::Running
            || id == 0
            || s.actors.contains_key(&id)
            || s.actors.len() >= s.max_actors
        {
            return Err("actor admission denied");
        }
        let epoch = s.next_actor_epoch;
        s.next_actor_epoch = s
            .next_actor_epoch
            .checked_add(1)
            .ok_or("actor epochs exhausted")?;
        let authority = Arc::new(());
        s.actors.insert(
            id,
            ActorState {
                epoch,
                authority: authority.clone(),
                pins: 0,
            },
        );
        Ok(ActorTicket {
            isolate: h.clone(),
            id,
            epoch,
            authority,
        })
    }
    pub fn pin_operation(
        &self,
        h: &IsolateHandle,
        actor: &ActorTicket,
    ) -> Result<ResourcePin, &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        check_actor(s, actor)?;
        if s.phase != Phase::Running || s.pins.len() >= s.max_pins {
            return Err("pin denied");
        }
        let id = s.next_pin;
        s.next_pin = s.next_pin.checked_add(1).ok_or("pin ids exhausted")?;
        let authority = Arc::new(());
        s.pins.insert(
            id,
            PinState {
                actor_id: actor.id,
                actor_epoch: actor.epoch,
                authority: authority.clone(),
            },
        );
        s.actors.get_mut(&actor.id).expect("validated actor").pins += 1;
        Ok(ResourcePin {
            isolate: h.clone(),
            actor: actor.clone(),
            id,
            authority,
        })
    }
    // The trusted native adapter calls this ONLY after physical callback retirement.
    pub fn retire_operation(
        &self,
        h: &IsolateHandle,
        actor: &ActorTicket,
        pin: &ResourcePin,
    ) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        check_actor(s, actor)?;
        if !Arc::ptr_eq(&pin.isolate.authority, &h.authority)
            || !Arc::ptr_eq(&pin.actor.authority, &actor.authority)
        {
            return Err("foreign pin");
        }
        let v = s.pins.get(&pin.id).ok_or("stale pin")?;
        if v.actor_id != actor.id
            || v.actor_epoch != actor.epoch
            || !Arc::ptr_eq(&v.authority, &pin.authority)
        {
            return Err("foreign pin");
        }
        s.pins.remove(&pin.id);
        s.actors.get_mut(&actor.id).expect("validated actor").pins -= 1;
        Ok(())
    }
    // Trusted scalar bridge into REAL compiled Cranelift code. The host
    // loads/stores tenant scalar state; generated native code receives only
    // two scalar values, NOT an unrestricted host-VM pointer. This is not
    // a general tenant-owned native heap, AOT, or untrusted JIT sandbox.
    //
    // Pin before waiting for the tenant's serial turn. Quiescence closes
    // queued admission but lets already-running turns finish and commit.
    pub fn invoke_trusted_step<F>(
        &self,
        h: &IsolateHandle,
        delta: i64,
        step: F,
    ) -> Result<i64, String>
    where
        F: FnOnce(i64, i64) -> Result<i64, String>,
    {
        // Nested entry on the same carrier could deadlock the serial-turn
        // mutex. Reject before pinning; only the outer invocation may reset it.
        thread_local! {
            static IN_NATIVE_TURN: std::cell::Cell<bool> =
                const { std::cell::Cell::new(false) };
        }
        let nested = IN_NATIVE_TURN.with(|flag| {
            if flag.get() {
                true
            } else {
                flag.set(true);
                false
            }
        });
        if nested {
            return Err("nested tenant native entry forbidden".into());
        }
        struct EntryGuard;
        impl Drop for EntryGuard {
            fn drop(&mut self) {
                IN_NATIVE_TURN.with(|flag| flag.set(false));
            }
        }
        let _entry_guard = EntryGuard;

        struct Pin<'a> {
            vm: &'a OresVM,
            handle: IsolateHandle,
        }
        impl Drop for Pin<'_> {
            fn drop(&mut self) {
                if let Ok(mut guard) = self.vm.inner.lock()
                    && let Ok(s) = guard.check_mut(&self.handle)
                {
                    s.active_guest_calls -= 1;
                }
            }
        }
        let turn = {
            let mut guard = self.inner.lock().map_err(|_| "poisoned host".to_owned())?;
            let state = guard.check_mut(h).map_err(str::to_owned)?;
            if state.phase != Phase::Running {
                return Err("guest admission closed".into());
            }
            if state.active_guest_calls >= 64 {
                return Err("guest quota exceeded".into());
            }
            state.active_guest_calls += 1;
            Arc::clone(&state.guest_turn)
        };
        let _pin = Pin {
            vm: self,
            handle: h.clone(),
        };
        let _serial = turn.lock().map_err(|_| "poisoned tenant turn".to_owned())?;
        let old = {
            let guard = self.inner.lock().map_err(|_| "poisoned host".to_owned())?;
            let state = guard.check(h).map_err(str::to_owned)?;
            if state.phase != Phase::Running {
                return Err("guest cancelled during quiescence".into());
            }
            state.scalar_state
        };
        let next = step(old, delta)?;
        {
            let mut guard = self.inner.lock().map_err(|_| "poisoned host".to_owned())?;
            let state = guard.check_mut(h).map_err(str::to_owned)?;
            if state.phase != Phase::Running && state.phase != Phase::Quiescing {
                return Err("guest has no live tenant to commit to".into());
            }
            state.scalar_state = next;
        }
        Ok(next)
    }
    pub fn retire_actor(&self, h: &IsolateHandle, actor: &ActorTicket) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        if check_actor(s, actor)?.pins != 0 {
            return Err("actor has native pins");
        }
        s.actors.remove(&actor.id);
        Ok(())
    }
    pub fn begin_shutdown(&self, h: &IsolateHandle) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        if s.phase != Phase::Created && s.phase != Phase::Running {
            return Err("shutdown phase");
        }
        s.phase = Phase::Quiescing;
        Ok(())
    }
    pub fn complete_shutdown(&self, h: &IsolateHandle) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check_mut(h)?;
        if s.phase != Phase::Quiescing
            || !s.actors.is_empty()
            || !s.pins.is_empty()
            || s.active_guest_calls != 0
        {
            return Err("isolate still has active work");
        }
        s.phase = Phase::Stopped;
        Ok(())
    }
    pub fn destroy(&self, h: &IsolateHandle) -> Result<(), &'static str> {
        let mut g = self.inner.lock().map_err(|_| "poisoned host")?;
        if g.check(h)?.phase != Phase::Stopped {
            return Err("isolate not stopped");
        }
        g.tenants.remove(&h.tenant);
        Ok(())
    }
    pub fn snapshot(&self, h: &IsolateHandle) -> Result<Snapshot, &'static str> {
        let g = self.inner.lock().map_err(|_| "poisoned host")?;
        let s = g.check(h)?;
        Ok(Snapshot {
            phase: s.phase,
            generation: s.generation,
            actors: s.actors.len(),
            active_pins: s.pins.len(),
            active_guest_calls: s.active_guest_calls,
            scalar_state: s.scalar_state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tenant_actor_pins_and_reincarnation() {
        let vm = OresVM::new(2).unwrap();
        let a = vm.create_isolate("a", 1, 1).unwrap();
        let b = vm.create_isolate("b", 1, 1).unwrap();
        assert!(vm.create_isolate("c", 1, 1).is_err());
        assert!(vm.create_isolate("a", 1, 1).is_err());
        vm.start(&a).unwrap();
        vm.start(&b).unwrap();
        let actor_a = vm.register_actor(&a, 7).unwrap();
        let actor_b = vm.register_actor(&b, 7).unwrap();
        let pin = vm.pin_operation(&a, &actor_a).unwrap();
        assert!(vm.retire_actor(&a, &actor_a).is_err());
        assert!(vm.retire_operation(&b, &actor_b, &pin).is_err());
        assert!(vm.pin_operation(&a, &actor_a).is_err());
        vm.begin_shutdown(&a).unwrap();
        assert!(vm.pin_operation(&a, &actor_a).is_err());
        assert!(vm.complete_shutdown(&a).is_err());
        vm.retire_operation(&a, &actor_a, &pin).unwrap();
        assert!(vm.retire_operation(&a, &actor_a, &pin).is_err());
        vm.retire_actor(&a, &actor_a).unwrap();
        vm.complete_shutdown(&a).unwrap();
        vm.destroy(&a).unwrap();
        assert!(vm.snapshot(&a).is_err());
        let new_a = vm.create_isolate("a", 1, 1).unwrap();
        assert_ne!(new_a.generation, a.generation);
        vm.start(&new_a).unwrap();
        assert!(vm.retire_actor(&new_a, &actor_a).is_err());
        vm.begin_shutdown(&new_a).unwrap();
        vm.complete_shutdown(&new_a).unwrap();
        vm.destroy(&new_a).unwrap();
        vm.retire_actor(&b, &actor_b).unwrap();
        vm.begin_shutdown(&b).unwrap();
        vm.complete_shutdown(&b).unwrap();
        vm.destroy(&b).unwrap();
    }
}

#[cfg(test)]
#[path = "isolate_integration_tests.rs"]
mod integration_tests;
