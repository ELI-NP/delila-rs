//! Test oxyroot TTree writing
//! Run with: cargo run --example test_oxyroot --features root

use oxyroot::{RootFile, WriterTree};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create ROOT file
    let mut file = RootFile::create("test_output.root")?;

    // Create tree
    let mut tree = WriterTree::new("events");

    // Sample data (simulating amax_viewer output)
    // Note: oxyroot uses i32/i64 for integers
    let energies: Vec<i32> = (0..100).map(|i: i32| i * 100).collect();
    let amax_values: Vec<i64> = (0..100).map(|i: i64| i * 50).collect();
    let timestamps: Vec<i64> = (0..100).map(|i: i64| i * 1000000).collect();

    // Add branches
    tree.new_branch("energy", energies.into_iter());
    tree.new_branch("amax", amax_values.into_iter());
    tree.new_branch("timestamp", timestamps.into_iter());

    // Write tree
    tree.write(&mut file)?;
    file.close()?;

    println!("Successfully wrote test_output.root");
    println!("Verify with: root -l test_output.root");
    println!("  events->Print()");
    println!("  events->Scan(\"energy:amax:timestamp\")");
    Ok(())
}
