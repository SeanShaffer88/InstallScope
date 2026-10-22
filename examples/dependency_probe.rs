//! Opt-in real download/bundle verification. Never launches an installer.
use setupscope_core::{dependencies as d, snapshot::Progress};
fn main() -> anyhow::Result<()> {
    let root =
        std::path::PathBuf::from(std::env::args().nth(1).expect("absolute new probe folder"));
    let ids = std::env::args().skip(2).collect::<Vec<_>>();
    let p = Progress::default();
    println!("{}", d::download(&root, &ids, &p)?);
    println!("{}", d::verify(&root, &ids, &p)?);
    let bundle = root.join("exported-bundle");
    println!("{}", d::export_bundle(&root, &ids, &bundle, true, &p)?);
    let imported = root.join("imported-library");
    println!("{}", d::import_bundle(&imported, &bundle, &p)?);
    println!("{}", d::verify(&imported, &ids, &p)?);
    for id in &ids {
        let review = d::review_install(&imported, id, false, &p)?;
        println!("REVIEW ONLY; NOT LAUNCHED\n{}", review.text());
    }
    Ok(())
}
