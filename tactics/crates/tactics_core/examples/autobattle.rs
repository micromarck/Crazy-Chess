//! Headless AI-vs-AI battles: a quick way to balance numbers and to show that
//! the rules run without any engine.
//!
//! ```sh
//! cargo run -p tactics_core --example autobattle -- 20
//! ```

use tactics_core::prelude::*;

fn main() {
    let battles: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let config = AiConfig::default();
    let (mut player_wins, mut enemy_wins, mut undecided) = (0, 0, 0);

    for seed in 0..battles {
        let mut battle = scenario::skirmish(seed);
        let mut actions = 0;
        while !battle.is_over() && battle.round() <= 60 {
            let action = ai::next_team_action(&battle, &config);
            battle.apply(action).expect("AI produced an illegal action");
            actions += 1;
        }
        let result = match battle.winner() {
            Some(Team::Player) => {
                player_wins += 1;
                "XCOM wins"
            }
            Some(Team::Enemy) => {
                enemy_wins += 1;
                "aliens win"
            }
            None => {
                undecided += 1;
                "undecided"
            }
        };
        println!("seed {seed:>3}: {result:<10} after {:>2} rounds, {actions:>4} actions", battle.round());
    }
    println!("\nXCOM {player_wins} / aliens {enemy_wins} / undecided {undecided}");
}
