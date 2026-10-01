//! Native Soroban-host regressions for #1178. No network calls or live funds.
//! Each assertion executes the real Distribute entrypoint and inspects host TTL.

use crate::{
    errors::DistributeError, limits, Distribute, DistributeClient, BUMP_AMOUNT,
    DEFAULT_MAX_DISTRIBUTE, INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_THRESHOLD, LEDGERS_PER_DAY,
    LIFETIME_THRESHOLD, VERSION_KEY,
};
use soroban_sdk::testutils::storage::Instance as _;
use soroban_sdk::testutils::{Address as _, EnvTestConfig, Events as _, Ledger as _};
use soroban_sdk::{Address, BytesN, Env, Map, Symbol, Val};

#[derive(Clone, Copy, Debug)]
enum View {
    Admin,
    Token,
    PendingAdmin,
    Paused,
    MaxDistribute,
    MaxBatchSize,
    Balance,
    VersionHash,
    Version,
}
const VIEWS: [View; 9] = [
    View::Admin,
    View::Token,
    View::PendingAdmin,
    View::Paused,
    View::MaxDistribute,
    View::MaxBatchSize,
    View::Balance,
    View::VersionHash,
    View::Version,
];

struct Fixture {
    env: Env,
    contract: Address,
    admin: Address,
    token: Address,
    nominee: Address,
    populated: bool,
}

impl Fixture {
    fn new(populated: bool) -> Self {
        let env = Env::new_with_config(EnvTestConfig {
            capture_snapshot_at_drop: false,
        });
        env.ledger().set_sequence_number(100);
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let nominee = Address::generate(&env);
        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let contract = env.register(Distribute, ());
        let client = DistributeClient::new(&env, &contract);
        client.init(&admin, &token);
        if populated {
            client.set_admin(&admin, &nominee);
            client.set_max_distribute(&admin, &1234);
            client.pause(&admin);
            // Seed only the read fixture's version field; no WASM upgrade is
            // needed to check Option<BytesN<32>> read semantics.
            env.as_contract(&contract, || {
                env.storage().instance().set(
                    &Symbol::new(&env, VERSION_KEY),
                    &BytesN::from_array(&env, &[7; 32]),
                );
            });
        }
        env.set_auths(&[]); // Views must work without any mocked authorization.
        let fixture = Self {
            env,
            contract,
            admin,
            token,
            nominee,
            populated,
        };
        fixture.keep_token_live();
        fixture
    }

    fn ttl(&self) -> u32 {
        self.env
            .as_contract(&self.contract, || self.env.storage().instance().get_ttl())
    }

    fn contents(&self) -> Map<Val, Val> {
        self.env
            .as_contract(&self.contract, || self.env.storage().instance().all())
    }

    fn advance(&self, ledgers: u32) {
        self.env
            .ledger()
            .set_sequence_number(self.env.ledger().sequence() + ledgers);
    }

    fn keep_token_live(&self) {
        // Isolate Distribute's lifetime from its external token dependency.
        self.env.as_contract(&self.token, || {
            self.env
                .storage()
                .instance()
                .extend_ttl(INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_AMOUNT * 2);
        });
    }

    fn read(&self, view: View) {
        let client = DistributeClient::new(&self.env, &self.contract);
        match view {
            View::Admin => assert_eq!(client.get_admin(), self.admin),
            View::Token => assert_eq!(client.get_usdc_token(), self.token),
            View::PendingAdmin => assert_eq!(
                client.get_pending_admin(),
                if self.populated {
                    Some(self.nominee.clone())
                } else {
                    None
                }
            ),
            View::Paused => assert_eq!(client.get_paused(), self.populated),
            View::MaxDistribute => assert_eq!(
                client.get_max_distribute(),
                if self.populated {
                    1234
                } else {
                    DEFAULT_MAX_DISTRIBUTE
                }
            ),
            View::MaxBatchSize => assert_eq!(client.get_max_batch_size(), limits::MAX_BATCH_SIZE),
            View::Balance => assert_eq!(client.balance(), 0),
            View::VersionHash => assert_eq!(
                client.get_version(),
                if self.populated {
                    Some(BytesN::from_array(&self.env, &[7; 32]))
                } else {
                    None
                }
            ),
            View::Version => assert_eq!(
                client.version(),
                soroban_sdk::String::from_str(&self.env, env!("CARGO_PKG_VERSION"))
            ),
        }
    }
}

#[test]
fn ttl_constants_are_single_source_and_match_workspace_policy() {
    assert_eq!(LEDGERS_PER_DAY, 17_280);
    assert_eq!(INSTANCE_BUMP_THRESHOLD, 17_280 * 30);
    assert_eq!(INSTANCE_BUMP_AMOUNT, 17_280 * 60);
    assert_eq!(LIFETIME_THRESHOLD, INSTANCE_BUMP_THRESHOLD);
    assert_eq!(BUMP_AMOUNT, INSTANCE_BUMP_AMOUNT);
    assert_eq!(limits::LIFETIME_THRESHOLD, INSTANCE_BUMP_THRESHOLD);
    assert_eq!(limits::BUMP_AMOUNT, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn ttl_init_extends_to_target() {
    let f = Fixture::new(false);
    assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT);
}

#[test]
fn ttl_read_boundary_matrix() {
    // SDK 22's host extends at TTL <= threshold, INCLUDING equality. TTL=0 is
    // the final live ledger, not an already archived instance.
    let remaining = [
        0,
        1,
        INSTANCE_BUMP_THRESHOLD - 1,
        INSTANCE_BUMP_THRESHOLD,
        INSTANCE_BUMP_THRESHOLD + 1,
        INSTANCE_BUMP_AMOUNT - 1,
        INSTANCE_BUMP_AMOUNT,
    ];
    for populated in [false, true] {
        for view in VIEWS {
            for ttl in remaining {
                let f = Fixture::new(populated);
                f.advance(INSTANCE_BUMP_AMOUNT - ttl);
                assert_eq!(f.ttl(), ttl);
                let before = f.contents();
                let event_count = f.env.events().all().len();
                let expected = if ttl <= INSTANCE_BUMP_THRESHOLD {
                    INSTANCE_BUMP_AMOUNT
                } else {
                    ttl
                };
                f.read(view);
                assert_eq!(
                    f.ttl(),
                    expected,
                    "view={view:?}, ttl={ttl}, populated={populated}"
                );
                assert_eq!(f.contents(), before, "view must not mutate configuration");
                assert_eq!(
                    f.env.events().all().len(),
                    event_count,
                    "view must not emit business events"
                );
                assert!(
                    f.env.auths().is_empty(),
                    "view must not require authorization"
                );
                f.read(view);
                assert_eq!(f.ttl(), expected, "same-ledger reads are idempotent");
            }
        }
    }
}

#[test]
fn ttl_read_survives_original_expiry() {
    for view in VIEWS {
        let f = Fixture::new(true);
        f.advance(INSTANCE_BUMP_AMOUNT - 1);
        f.read(view);
        assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT);
        // Pass the original expiry without any Distribute business writes.
        f.advance(INSTANCE_BUMP_THRESHOLD + 2);
        assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD - 2);
        f.read(view);
        assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT);
    }
}

#[test]
fn ttl_read_never_shortens_an_external_extension() {
    for view in VIEWS {
        let f = Fixture::new(false);
        f.env.as_contract(&f.contract, || {
            f.env
                .storage()
                .instance()
                .extend_ttl(INSTANCE_BUMP_AMOUNT, INSTANCE_BUMP_AMOUNT * 2);
        });
        f.read(view);
        assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT * 2);
    }
}

#[test]
fn ttl_instances_are_isolated() {
    let f = Fixture::new(false);
    let second = f.env.register(Distribute, ());
    DistributeClient::new(&f.env, &second).init(&f.admin, &f.token);
    f.advance(INSTANCE_BUMP_AMOUNT - 1);
    f.read(View::Admin);
    assert_eq!(f.ttl(), INSTANCE_BUMP_AMOUNT);
    assert_eq!(
        f.env
            .as_contract(&second, || f.env.storage().instance().get_ttl()),
        1
    );
}

#[test]
fn ttl_failed_uninitialized_reads_preserve_error_and_storage() {
    let env = Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    });
    let contract = env.register(Distribute, ());
    let client = DistributeClient::new(&env, &contract);
    let before = env.as_contract(&contract, || {
        (
            env.storage().instance().get_ttl(),
            env.storage().instance().all(),
        )
    });
    assert_eq!(
        client.try_get_admin(),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            DistributeError::NotInitialized as u32
        )))
    );
    assert_eq!(
        client.try_get_usdc_token(),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            DistributeError::NotInitialized as u32
        )))
    );
    assert_eq!(
        client.try_balance(),
        Err(Ok(soroban_sdk::Error::from_contract_error(
            DistributeError::NotInitialized as u32
        )))
    );
    let after = env.as_contract(&contract, || {
        (
            env.storage().instance().get_ttl(),
            env.storage().instance().all(),
        )
    });
    assert_eq!(before, after);
}

#[test]
fn ttl_deterministic_read_schedules_match_host() {
    // 128 independent histories, 16 actual contract invocations in each.
    for seed in 0..128_u64 {
        let f = Fixture::new(seed % 2 == 0);
        let before = f.contents();
        let mut rng = seed + 1;
        let mut expected = INSTANCE_BUMP_AMOUNT;
        for step in 0..16 {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let elapsed = (rng >> 32) as u32 % (expected + 1);
            f.keep_token_live();
            f.advance(elapsed);
            expected -= elapsed;
            let view = VIEWS[(rng as usize) % VIEWS.len()];
            f.read(view);
            if expected <= INSTANCE_BUMP_THRESHOLD {
                expected = INSTANCE_BUMP_AMOUNT;
            }
            assert_eq!(f.ttl(), expected, "seed={seed}, step={step}, view={view:?}");
            assert_eq!(f.contents(), before);
        }
    }
}
