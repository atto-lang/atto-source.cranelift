use super::*;
#[test]
fn native_generated_scalar_step_is_tenant_private() {
    let program =
        crate::frontend::parse("fnc main(old:int,delta:int):int{return old+delta;}").unwrap();
    let compiled = crate::backend::compile(&program).unwrap();
    let vm = OresVM::new(2).unwrap();
    let a: TenantVMHandle = vm.create_tenant_vm("tenant-a", 2, 2).unwrap();
    for invalid in [".", "..", "../escape", "bad:tenant", "bad/tenant"] {
        assert!(vm.create_tenant_vm(invalid, 1, 1).is_err());
    }
    let b = vm.create_isolate("tenant-b", 2, 2).unwrap();
    vm.start(&a).unwrap();
    vm.start(&b).unwrap();
    // Nested entry must fail immediately rather than deadlock the same
    // carrier while holding the tenant's serial-turn lock.
    assert!(
        vm.invoke_trusted_step(&a, 1, |_, _| {
            vm.invoke_trusted_step(&a, 1, |_, _| Ok(1))
        })
        .is_err()
    );
    assert_eq!(vm.snapshot(&a).unwrap().active_guest_calls, 0);
    assert_eq!(compiled.run_for_isolate(&vm, &a, -1).unwrap(), -1);
    assert_eq!(compiled.run_for_isolate(&vm, &b, 100).unwrap(), 100);
    assert_eq!(compiled.run_for_isolate(&vm, &a, -2).unwrap(), -3);
    assert_eq!(compiled.run_for_isolate(&vm, &b, 2).unwrap(), 102);
    assert_eq!(vm.snapshot(&a).unwrap().scalar_state, -3);
    assert_eq!(vm.snapshot(&b).unwrap().scalar_state, 102);
    assert!(compiled.run_for_isolate(&vm, &b, i64::MAX).is_err());
    assert_eq!(vm.snapshot(&b).unwrap().scalar_state, 102);
    vm.begin_shutdown(&a).unwrap();
    assert!(compiled.run_for_isolate(&vm, &a, 1).is_err());
    assert_eq!(compiled.run_for_isolate(&vm, &b, 1).unwrap(), 103);
    vm.complete_shutdown(&a).unwrap();
    vm.destroy(&a).unwrap();
    let fresh = vm.create_isolate("tenant-a", 2, 2).unwrap();
    vm.start(&fresh).unwrap();
    assert_eq!(compiled.run_for_isolate(&vm, &fresh, 8).unwrap(), 8);
    assert!(vm.snapshot(&a).is_err());
    vm.begin_shutdown(&fresh).unwrap();
    vm.complete_shutdown(&fresh).unwrap();
    vm.destroy(&fresh).unwrap();
    vm.begin_shutdown(&b).unwrap();
    vm.complete_shutdown(&b).unwrap();
    vm.destroy(&b).unwrap();
}
#[test]
fn running_guest_turn_blocks_isolate_retirement() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let vm = Arc::new(OresVM::new(1).unwrap());
    let a = vm.create_isolate("busy", 1, 1).unwrap();
    vm.start(&a).unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let allow_exit = Arc::new(AtomicBool::new(false));
    let thread_vm = Arc::clone(&vm);
    let thread_a = a.clone();
    let e = Arc::clone(&entered);
    let x = Arc::clone(&allow_exit);
    let t = std::thread::spawn(move || {
        thread_vm
            .invoke_trusted_step(&thread_a, 4, |old, delta| {
                e.store(true, Ordering::Release);
                while !x.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                Ok(old + delta)
            })
            .unwrap()
    });
    while !entered.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    vm.begin_shutdown(&a).unwrap();
    assert_eq!(vm.snapshot(&a).unwrap().active_guest_calls, 1);
    assert!(vm.complete_shutdown(&a).is_err());
    allow_exit.store(true, Ordering::Release);
    assert_eq!(t.join().unwrap(), 4);
    assert_eq!(vm.snapshot(&a).unwrap().active_guest_calls, 0);
    assert_eq!(vm.snapshot(&a).unwrap().scalar_state, 4);
    vm.complete_shutdown(&a).unwrap();
    vm.destroy(&a).unwrap();
}
