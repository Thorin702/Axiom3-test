//! A REGISTRY of specialized scientific data tools — keyless public JSON APIs made first-class so the
//! research agent calls a clean tool (reliable) instead of hand-writing fetch+parse code each time
//! (which weaker models get wrong). Each tool: a name, the JSON arg key it reads, a one-line prompt
//! description, and an async handler that returns a text observation. Add a tool = add a `SciTool`
//! entry + a match arm in [`run_sci_tool`]. Bounded + fail-soft (an upstream error is surfaced, never
//! a panic). The QUERY is untrusted (model-chosen) so it only ever flows through URL-encoded params.
use crate::http::json_bounded;
use serde_json::Value;

const UA: &str = "Mozilla/5.0 (compatible; Axiom3Research/1.0)";

/// One registered science tool (metadata for the prompt + the arg key its JSON carries).
pub struct SciTool {
    pub name: &'static str,
    pub arg: &'static str,
    pub desc: &'static str,
}

/// The registry. The coding loop lists these in the prompt and dispatches to [`run_sci_tool`].
pub fn sci_tools() -> &'static [SciTool] {
    &[
        SciTool { name: "pubchem", arg: "name", desc: "{\"tool\":\"pubchem\",\"name\":\"<compound name or CID>\"} — look up a chemical compound in PubChem: canonical SMILES, molecular formula, weight, IUPAC name. (Then you can render it with a ```smiles block.)" },
        SciTool { name: "uniprot", arg: "query", desc: "{\"tool\":\"uniprot\",\"query\":\"<protein/gene + organism, e.g. 'TP53 human'>\"} — look up a protein in UniProt: accession, names, organism, length, and a function summary." },
        SciTool { name: "pdb", arg: "id", desc: "{\"tool\":\"pdb\",\"id\":\"<4-char PDB id, e.g. 1CRN>\"} — fetch a protein structure's metadata from the RCSB PDB: title, method, resolution, organism. (Then render it with a ```protein block.)" },
        SciTool { name: "gene", arg: "query", desc: "{\"tool\":\"gene\",\"query\":\"<gene symbol, e.g. BRCA1 human>\"} — look up a gene via mygene.info: official symbol, name, a summary, genomic location, Entrez id." },
        SciTool { name: "trials", arg: "query", desc: "{\"tool\":\"trials\",\"query\":\"<condition or intervention>\"} — search ClinicalTrials.gov: NCT id, title, status, phase, condition — for the clinical/translational state of a therapy." },
        SciTool { name: "europepmc", arg: "query", desc: "{\"tool\":\"europepmc\",\"query\":\"<search terms>\"} — search Europe PMC (papers AND preprints, incl. bioRxiv/medRxiv) — title, authors, year, DOI, source. Broader than litsearch; use it to catch preprints." },
    ]
}

pub fn is_sci_tool(name: &str) -> bool {
    sci_tools().iter().any(|t| t.name == name)
}

pub fn sci_tool_arg(name: &str) -> &'static str {
    sci_tools().iter().find(|t| t.name == name).map(|t| t.arg).unwrap_or("query")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Dispatch a science tool by name. `arg` is the model-supplied value (a name / id / query).
pub async fn run_sci_tool(name: &str, arg: &str) -> String {
    let arg = arg.trim();
    if arg.is_empty() {
        return format!("ERROR: {name} needs a non-empty argument");
    }
    let r = match name {
        "pubchem" => pubchem(arg).await,
        "uniprot" => uniprot(arg).await,
        "pdb" => pdb(arg).await,
        "gene" => gene(arg).await,
        "trials" => trials(arg).await,
        "europepmc" => europepmc(arg).await,
        other => Err(anyhow::anyhow!("unknown science tool: {other}")),
    };
    r.unwrap_or_else(|e| format!("ERROR: {name} failed: {e}"))
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

async fn pubchem(name: &str) -> anyhow::Result<String> {
    // name → CID → properties. PubChem PUG-REST, JSON.
    // PubChem (2025) renamed the SMILES properties: request the new names AND the old ones, then take
    // whichever the server fills, so a SMILES always comes back for the ```smiles renderer.
    let url = format!(
        "https://pubchem.ncbi.nlm.nih.gov/rest/pug/compound/name/{}/property/MolecularFormula,MolecularWeight,SMILES,ConnectivitySMILES,IsomericSMILES,CanonicalSMILES,IUPACName/JSON",
        urlq(name)
    );
    let resp = client().get(&url).send().await.map_err(anyerr)?;
    if resp.status().as_u16() == 404 {
        return Ok(format!("(PubChem: no compound named '{name}')"));
    }
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let props = v.pointer("/PropertyTable/Properties/0").cloned().unwrap_or(Value::Null);
    if props.is_null() {
        return Ok(format!("(PubChem: no properties for '{name}')"));
    }
    let smiles = ["SMILES", "IsomericSMILES", "ConnectivitySMILES", "CanonicalSMILES"]
        .iter()
        .map(|k| s(&props, k))
        .find(|x| !x.is_empty())
        .unwrap_or_default();
    Ok(format!(
        "PubChem · {name}\n  SMILES: {}\n  分子式 Formula: {}\n  分子量 MW: {}\n  IUPAC: {}\n(可用 ```smiles\\n{}\\n``` 渲染结构)",
        smiles,
        s(&props, "MolecularFormula"),
        props.get("MolecularWeight").map(|x| x.to_string()).unwrap_or_default().trim_matches('"'),
        s(&props, "IUPACName"),
        smiles
    ))
}

async fn uniprot(query: &str) -> anyhow::Result<String> {
    let url = "https://rest.uniprot.org/uniprotkb/search";
    let resp = client()
        .get(url)
        .query(&[
            ("query", query),
            ("format", "json"),
            ("size", "3"),
            ("fields", "accession,id,protein_name,gene_names,organism_name,length,cc_function"),
        ])
        .send()
        .await
        .map_err(anyerr)?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let empty = vec![];
    let results = v.get("results").and_then(|x| x.as_array()).unwrap_or(&empty);
    if results.is_empty() {
        return Ok(format!("(UniProt: no hit for '{query}')"));
    }
    let mut out = String::new();
    for r in results.iter().take(3) {
        let acc = s(r, "primaryAccession");
        let name = r.pointer("/proteinDescription/recommendedName/fullName/value").and_then(|x| x.as_str()).unwrap_or("");
        let org = r.pointer("/organism/scientificName").and_then(|x| x.as_str()).unwrap_or("");
        let len = r.get("sequence").and_then(|sq| sq.get("length")).map(|x| x.to_string()).unwrap_or_default();
        let func = r
            .pointer("/comments")
            .and_then(|c| c.as_array())
            .and_then(|cs| cs.iter().find(|c| s(c, "commentType") == "FUNCTION"))
            .and_then(|c| c.pointer("/texts/0/value"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        out.push_str(&format!(
            "[{acc}] {name} · {org} · {len} aa\n    功能 Function: {}\n",
            func.chars().take(320).collect::<String>()
        ));
    }
    Ok(format!("UniProt · {query}\n{out}"))
}

async fn pdb(id: &str) -> anyhow::Result<String> {
    let id = id.trim().to_ascii_uppercase();
    if id.len() != 4 {
        return Ok("(PDB id must be 4 characters, e.g. 1CRN)".to_string());
    }
    let url = format!("https://data.rcsb.org/rest/v1/core/entry/{id}");
    let resp = client().get(&url).send().await.map_err(anyerr)?;
    if resp.status().as_u16() == 404 {
        return Ok(format!("(PDB: no entry {id})"));
    }
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let title = v.pointer("/struct/title").and_then(|x| x.as_str()).unwrap_or("");
    let method = v.pointer("/exptl/0/method").and_then(|x| x.as_str()).unwrap_or("");
    let res = v.pointer("/rcsb_entry_info/resolution_combined/0").map(|x| x.to_string()).unwrap_or_default();
    let date = v.pointer("/rcsb_accession_info/deposit_date").and_then(|x| x.as_str()).unwrap_or("");
    Ok(format!(
        "PDB · {id}\n  标题 Title: {title}\n  方法 Method: {method}\n  分辨率 Resolution: {res} Å\n  沉积日期 Deposited: {date}\n(可用 ```protein\\n{id}\\n``` 渲染 3D 结构)"
    ))
}

async fn gene(query: &str) -> anyhow::Result<String> {
    let url = "https://mygene.info/v3/query";
    let resp = client()
        .get(url)
        .query(&[("q", query), ("fields", "symbol,name,summary,genomic_pos,entrezgene,taxid"), ("size", "3")])
        .send()
        .await
        .map_err(anyerr)?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let empty = vec![];
    let hits = v.get("hits").and_then(|x| x.as_array()).unwrap_or(&empty);
    if hits.is_empty() {
        return Ok(format!("(mygene: no gene for '{query}')"));
    }
    let mut out = String::new();
    for h in hits.iter().take(3) {
        let pos = h.get("genomic_pos");
        let loc = pos
            .and_then(|p| p.as_object().or_else(|| p.as_array().and_then(|a| a.first()).and_then(|x| x.as_object())))
            .map(|o| format!("chr{}:{}-{}", o.get("chr").map(|x| x.to_string()).unwrap_or_default().trim_matches('"'), o.get("start").map(|x| x.to_string()).unwrap_or_default(), o.get("end").map(|x| x.to_string()).unwrap_or_default()))
            .unwrap_or_default();
        out.push_str(&format!(
            "{} — {} (Entrez {})  {}\n    {}\n",
            s(h, "symbol"),
            s(h, "name"),
            h.get("entrezgene").map(|x| x.to_string()).unwrap_or_default().trim_matches('"'),
            loc,
            s(h, "summary").chars().take(320).collect::<String>()
        ));
    }
    Ok(format!("mygene · {query}\n{out}"))
}

async fn trials(query: &str) -> anyhow::Result<String> {
    let url = "https://clinicaltrials.gov/api/v2/studies";
    let resp = client()
        .get(url)
        .query(&[("query.term", query), ("pageSize", "5")])
        .send()
        .await
        .map_err(anyerr)?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let empty = vec![];
    let studies = v.get("studies").and_then(|x| x.as_array()).unwrap_or(&empty);
    if studies.is_empty() {
        return Ok(format!("(ClinicalTrials: no study for '{query}')"));
    }
    let mut out = String::new();
    for st in studies.iter().take(5) {
        let p = st.pointer("/protocolSection").unwrap_or(&Value::Null);
        let nct = p.pointer("/identificationModule/nctId").and_then(|x| x.as_str()).unwrap_or("");
        let title = p.pointer("/identificationModule/briefTitle").and_then(|x| x.as_str()).unwrap_or("");
        let status = p.pointer("/statusModule/overallStatus").and_then(|x| x.as_str()).unwrap_or("");
        let phase = p.pointer("/designModule/phases/0").and_then(|x| x.as_str()).unwrap_or("N/A");
        out.push_str(&format!("[{nct}] {title}\n    状态 {status} · 期 {phase}\n", ));
    }
    Ok(format!("ClinicalTrials.gov · {query}\n{out}"))
}

async fn europepmc(query: &str) -> anyhow::Result<String> {
    let url = "https://www.ebi.ac.uk/europepmc/webservices/rest/search";
    let resp = client()
        .get(url)
        .query(&[("query", query), ("format", "json"), ("pageSize", "6"), ("resultType", "lite")])
        .send()
        .await
        .map_err(anyerr)?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status().as_u16());
    }
    let v = json_bounded(resp).await?;
    let empty = vec![];
    let results = v.pointer("/resultList/result").and_then(|x| x.as_array()).unwrap_or(&empty);
    if results.is_empty() {
        return Ok(format!("(Europe PMC: no result for '{query}')"));
    }
    let mut out = String::new();
    for (i, r) in results.iter().take(6).enumerate() {
        let title = s(r, "title");
        let auth = s(r, "authorString");
        let year = s(r, "pubYear");
        let src = s(r, "source"); // MED / PPR (preprint) / etc.
        let doi = s(r, "doi");
        out.push_str(&format!(
            "[{}] {title} ({year}) — {}\n    {}{}\n",
            i + 1,
            auth.chars().take(80).collect::<String>(),
            if src == "PPR" { "预印本 preprint · " } else { "" },
            if doi.is_empty() { String::new() } else { format!("DOI: {doi}") }
        ));
    }
    Ok(format!("Europe PMC · {query}\n{out}"))
}

fn urlq(s: &str) -> String {
    // percent-encode a path segment (compound names can contain spaces/specials).
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn anyerr(e: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_is_wellformed() {
        for t in sci_tools() {
            assert!(!t.name.is_empty() && !t.arg.is_empty() && t.desc.contains(t.name));
            assert!(is_sci_tool(t.name));
        }
        assert!(!is_sci_tool("python"));
        assert_eq!(sci_tool_arg("pdb"), "id");
    }
    #[test]
    fn urlq_encodes() {
        assert_eq!(urlq("acetyl salicylic"), "acetyl%20salicylic");
    }
}
