#[test]
fn probe_near_parameters_matches_mock() {
    use near_parameters::config_store::RuntimeConfigStore;
    use near_parameters::cost::ExtCosts;
    use near_primitives_core::gas::Gas;
    let store = RuntimeConfigStore::new(None);
    let cfg = store.get_config(155);
    let ec = &cfg.wasm_config.ext_costs;
    // SAFETY_MULTIPLIER=3 applied to base table (near-parameters 0.37.3 EnumMap API)
    assert_eq!(
        ec.gas_cost(ExtCosts::log_base),
        Gas::from_gas(3 * 1_181_104_350)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::storage_write_base),
        Gas::from_gas(3 * 21_398_912_000)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::storage_write_key_byte),
        Gas::from_gas(3 * 23_494_289)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::read_register_base),
        Gas::from_gas(3 * 839_055_062)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::read_register_byte),
        Gas::from_gas(3 * 32_854)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::read_memory_base),
        Gas::from_gas(3 * 869_954_400)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::read_memory_byte),
        Gas::from_gas(3 * 1_267_111)
    );
    assert_eq!(
        ec.gas_cost(ExtCosts::log_byte),
        Gas::from_gas(3 * 4_399_597)
    );
}
