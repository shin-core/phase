//! Issue #5928 — Iron Mastiff's "roll a d20 for each player being attacked and
//! ignore all but the highest roll" attack trigger with a d20 outcome table.
//!
//! Oracle (Iron Mastiff, Scryfall-verified):
//! > Whenever this creature attacks, roll a d20 for each player being attacked
//! > and ignore all but the highest roll.
//! > 1—9   | This creature deals damage equal to its power to you.
//! > 10—19 | This creature deals damage equal to its power to defending player.
//! > 20    | This creature deals damage equal to its power to each opponent.
//!
//! Before the fix, the attacks-trigger effect parsed to
//! `Unimplemented("roll", …)` and the three outcome rows landed as three
//! detached abilities, so the roll never happened and no damage was dealt.
//!
//! These tests drive real combat (Iron Mastiff, a 4/4, attacks P1), force a
//! roll into each outcome bucket by scanning seeds, and assert that the correct
//! player's life dropped by 4 (the mastiff's power) and the OTHER rows' targets
//! were untouched. Because "ignore all but the highest roll" collapses the
//! table to a single lookup, exactly one row fires per attack.

use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::WaitingFor;
use engine::types::phase::Phase;

use engine::parser::oracle::parse_oracle_text;
use engine::types::ability::{DieRollAggregate, Effect, PlayerFilter, QuantityExpr, QuantityRef};
use engine::types::triggers::TriggerMode;

const IRON_MASTIFF_ORACLE: &str = "Whenever this creature attacks, roll a d20 for each player \
being attacked and ignore all but the highest roll.\n1—9 | This creature deals damage equal to \
its power to you.\n10—19 | This creature deals damage equal to its power to defending player.\n20 \
| This creature deals damage equal to its power to each opponent.";

const MASTIFF_POWER: i32 = 4;

/// Build a two-player scenario at `seed`, declare Iron Mastiff (a 4/4) attacking
/// P1, resolve the on-stack attacks trigger (roll + outcome-table damage) by
/// passing priority ourselves so we capture the emitted `DieRolled` event, and
/// STOP before combat damage so life deltas reflect the trigger alone (the 4/4's
/// own 4 combat damage to P1 would otherwise confound the "to defending player"
/// row). Returns the runner and the actual d20 result that drove the table.
fn attack_and_resolve(seed: u64) -> (GameRunner, u8) {
    let mut scenario = GameScenario::new_n_player(2, seed);
    scenario.at_phase(Phase::PreCombatMain);
    let mastiff = scenario
        .add_creature_from_oracle(P0, "Iron Mastiff", 4, 4, IRON_MASTIFF_ORACLE)
        .id();
    let mut runner = scenario.build();

    // Advance to the declare-attackers step and declare the mastiff attacking P1.
    runner.pass_both_players();
    runner
        .act(GameAction::DeclareAttackers {
            attacks: vec![(mastiff, AttackTarget::Player(P1))],
            bands: vec![],
        })
        .expect("DeclareAttackers should succeed");

    // The attacks trigger is now on the stack. Resolve it (and only it) by
    // passing priority, capturing events, until the stack is empty again — which
    // happens during the declare-attackers priority window, before combat damage.
    let mut all_events: Vec<GameEvent> = Vec::new();
    for _ in 0..40 {
        if !runner.state().stack.is_empty()
            || matches!(runner.state().waiting_for, WaitingFor::Priority { .. })
        {
            match runner.act(GameAction::PassPriority) {
                Ok(result) => all_events.extend(result.events),
                Err(_) => break,
            }
            // Stop once the trigger has resolved and the stack is drained.
            if runner.state().stack.is_empty()
                && all_events
                    .iter()
                    .any(|e| matches!(e, GameEvent::DieRolled { sides: 20, .. }))
            {
                break;
            }
        } else {
            break;
        }
    }

    let rolled = all_events
        .iter()
        .find_map(|e| match e {
            GameEvent::DieRolled {
                result, sides: 20, ..
            } => result.map(u8::from),
            _ => None,
        })
        .expect("Iron Mastiff must roll a d20 when it attacks");
    assert!(
        (1..=20).contains(&rolled),
        "d20 result out of range: {rolled}"
    );
    (runner, rolled)
}

/// Scan seeds until the mastiff's single d20 roll lands in `[lo, hi]`, so the
/// outcome row under test deterministically fires.
fn resolve_with_roll_in(lo: u8, hi: u8) -> (GameRunner, u8) {
    for seed in 0..2000u64 {
        let (runner, rolled) = attack_and_resolve(seed);
        if (lo..=hi).contains(&rolled) {
            return (runner, rolled);
        }
    }
    panic!("no seed in 0..2000 produced a d20 roll in {lo}..={hi}");
}

#[test]
fn iron_mastiff_low_roll_deals_power_to_controller() {
    // 1—9: "deals damage equal to its power to you" — the controller (P0).
    let (runner, rolled) = resolve_with_roll_in(1, 9);
    assert!(
        (1..=9).contains(&rolled),
        "expected a low roll, got {rolled}"
    );

    // CR 119.3: P0 (you) took 4; P1 (defending player) untouched.
    assert_eq!(
        runner.state().players[P0.0 as usize].life,
        20 - MASTIFF_POWER,
        "low roll must deal the mastiff's power to its controller (roll = {rolled})"
    );
    assert_eq!(
        runner.state().players[P1.0 as usize].life,
        20,
        "low roll must NOT touch the defending player (roll = {rolled})"
    );
}

#[test]
fn iron_mastiff_mid_roll_deals_power_to_defending_player() {
    // 10—19: "deals damage equal to its power to defending player" (P1).
    let (runner, rolled) = resolve_with_roll_in(10, 19);
    assert!(
        (10..=19).contains(&rolled),
        "expected a mid roll, got {rolled}"
    );

    assert_eq!(
        runner.state().players[P1.0 as usize].life,
        20 - MASTIFF_POWER,
        "mid roll must deal the mastiff's power to the defending player (roll = {rolled})"
    );
    assert_eq!(
        runner.state().players[P0.0 as usize].life,
        20,
        "mid roll must NOT touch the controller (roll = {rolled})"
    );
}

#[test]
fn iron_mastiff_max_roll_deals_power_to_each_opponent() {
    // 20: "deals damage equal to its power to each opponent" — P1 is P0's only
    // opponent in a two-player game.
    let (runner, rolled) = resolve_with_roll_in(20, 20);
    assert_eq!(rolled, 20, "expected a natural 20");

    assert_eq!(
        runner.state().players[P1.0 as usize].life,
        20 - MASTIFF_POWER,
        "natural 20 must deal the mastiff's power to each opponent (roll = {rolled})"
    );
    // The controller is not an opponent, so it must not be hit.
    assert_eq!(
        runner.state().players[P0.0 as usize].life,
        20,
        "natural 20 must NOT touch the controller (each opponent excludes you)"
    );
}

/// Guard: the attacks trigger genuinely produces a roll (proving the fixture
/// reaches the RollDie arm and past any Unimplemented short-circuit). Without a
/// working parse the `attack_and_resolve` helper would panic on the missing
/// `DieRolled` event, so a passing outcome test above already proves reach — this
/// test makes the reach-guard explicit and independent of the outcome bucket.
#[test]
fn iron_mastiff_attack_actually_rolls_a_d20() {
    let (_runner, rolled) = attack_and_resolve(7);
    assert!(
        (1..=20).contains(&rolled),
        "attacks trigger must roll a real d20 (got {rolled})"
    );
}

/// Parser shape: Iron Mastiff's verbatim Oracle text parses to a single
/// `TriggerMode::Attacks` trigger whose execute effect is
/// `RollDie { sides: 20, keep: Highest, count: <players being attacked>,
/// results: [1—9, 10—19, 20] }`, each branch a real `DealDamage` (never
/// `Unimplemented`). This is the AST guard behind the runtime tests above.
#[test]
fn iron_mastiff_parses_to_keep_highest_roll_table() {
    let parsed = parse_oracle_text(
        IRON_MASTIFF_ORACLE,
        "Iron Mastiff",
        &[],
        &["Artifact".to_string(), "Creature".to_string()],
        &["Dog".to_string()],
    );

    // Exactly one attacks trigger, no detached outcome-row abilities and no
    // Unimplemented fallthrough.
    assert_eq!(
        parsed.triggers.len(),
        1,
        "expected one attacks trigger, got {:#?}",
        parsed.triggers
    );
    assert!(
        parsed.abilities.is_empty(),
        "outcome rows must attach to the roll, not become detached abilities: {:#?}",
        parsed.abilities
    );

    let trigger = &parsed.triggers[0];
    assert!(
        matches!(trigger.mode, TriggerMode::Attacks),
        "expected TriggerMode::Attacks, got {:?}",
        trigger.mode
    );

    let execute = trigger
        .execute
        .as_ref()
        .expect("attacks trigger must have an execute effect");

    match &*execute.effect {
        Effect::RollDie {
            count,
            sides,
            results,
            modifier,
            keep,
        } => {
            assert_eq!(*sides, 20, "Iron Mastiff rolls a d20");
            assert_eq!(
                *keep,
                DieRollAggregate::Highest,
                "'ignore all but the highest roll' must set keep-highest"
            );
            assert!(modifier.is_none(), "no add/subtract modifier on this roll");
            // "for each player being attacked" → players this creature is
            // attacking this combat (CR 508.6).
            assert!(
                matches!(
                    count,
                    QuantityExpr::Ref {
                        qty: QuantityRef::PlayerCount {
                            filter: PlayerFilter::OpponentAttacked { .. }
                        }
                    }
                ),
                "count must be the attacked-players count, got {count:?}"
            );

            // Three outcome rows with the printed ranges, each a DealDamage.
            let ranges: Vec<(u8, u8)> = results.iter().map(|b| (b.min, b.max)).collect();
            assert_eq!(
                ranges,
                vec![(1, 9), (10, 19), (20, 20)],
                "outcome-table ranges must be 1—9, 10—19, 20"
            );
            // Each row deals damage; "to you"/"to defending player" lower to
            // `DealDamage`, "to each opponent" to `DamageEachPlayer` (CR 120).
            // Neither may be `Unimplemented`.
            for branch in results {
                assert!(
                    matches!(
                        &*branch.effect.effect,
                        Effect::DealDamage { .. } | Effect::DamageEachPlayer { .. }
                    ),
                    "branch {}—{} must deal damage (not Unimplemented), got {:?}",
                    branch.min,
                    branch.max,
                    branch.effect.effect
                );
            }
        }
        other => panic!("expected RollDie execute effect, got {other:?}"),
    }
}
