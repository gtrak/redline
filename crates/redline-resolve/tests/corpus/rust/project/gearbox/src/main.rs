//! The gearbox sample binary: deep `::` chains, turbofish, and generic
//! arguments at call sites.

use gearbox::engine::{Engine, piston};
use gearbox::{Drivetrain, gear_lookup};
use widgets::gear::Gear as DriveGear;

fn main() {
    // Deep `::` chain across the module tree: gearbox::core::math::sum_sq.
    let energy = gearbox::core::math::sum_sq(&[3.0, 4.0]);

    // Deep chain into the widgets module tree: widgets::gear::Gear.
    let low = DriveGear::new(20, 2);
    let high = widgets::gear::Gear::new(60, 2);
    let engine = Engine::assemble(low, high);

    // Turbofish pins HashMap's key/value types at construction.
    let ratios = vec![
        widgets::GearRatio {
            teeth: low.teeth,
            module: low.module,
        },
        widgets::GearRatio {
            teeth: high.teeth,
            module: high.module,
        },
    ];
    let table = gear_lookup(&ratios);
    let first = table.get(&20).copied().unwrap_or(0.0);

    // Generic argument at a call site (the `T: Display` bound is exercised).
    let report = piston::describe::<f64>(engine.output.teeth, engine.ratio());

    let mut drive = Drivetrain {
        stages: vec![low, high],
        name: String::from("sample"),
    };
    drive.add_stage(2, DriveGear::new(90, 2));

    println!(
        "energy {energy} first {first} report {report} ratio {}",
        drive.total_ratio()
    );
}
