use super::*;

#[test]
fn units_run_update_daily() {
    let [(service, s), (timer, t)] = units(
        Path::new("/home/u/.nix-profile/bin/protonhub"),
        "Environment=PROTONHUB_REGISTRY=/src/protonhub/registry\n",
    );
    assert_eq!((service, timer), (SERVICE, TIMER));
    assert!(s.contains("\nExecStart=/home/u/.nix-profile/bin/protonhub update --notify\n"));
    assert!(s.contains("\nEnvironment=PROTONHUB_REGISTRY=/src/protonhub/registry\n"));
    assert!(t.contains("\nOnCalendar=daily\n") && t.contains("\nWantedBy=timers.target\n"));
}
