//! Live check of the specialized science-database tools.
//!   cargo run -p axiom-runtime --example sci_probe
use axiom_runtime::scitools::run_sci_tool;

#[tokio::main]
async fn main() {
    for (tool, arg) in [
        ("pubchem", "aspirin"),
        ("uniprot", "TP53 human"),
        ("pdb", "1CRN"),
        ("gene", "BRCA1 human"),
        ("europepmc", "senescence immune evasion"),
        ("trials", "senolytic"),
    ] {
        println!("\n========== {tool}({arg}) ==========");
        let out = run_sci_tool(tool, arg).await;
        println!("{}", out.chars().take(700).collect::<String>());
    }
}
