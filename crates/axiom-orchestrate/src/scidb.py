"""
scidb — Axiom3 pre-configured scientific-database toolkit.

Out-of-the-box structured access to the major authoritative biology / chemistry
databases + the literature, so an analysis never hand-rolls a fragile requests
call. Every function returns clean Python dicts/lists that ALWAYS carry the
record's canonical accession and a `source_url`, so results are citable ([Sn])
and verifiable — this is a synthesis tool, so it surfaces real records and NEVER
fabricates one. On failure it returns {"error": ...} (or raises) — it never
invents data to fill a gap.

Covered hubs (one Entrez wrapper alone reaches ~40 NCBI databases):
  Literature : Europe PMC, PubMed (E-utilities), Crossref, Semantic Scholar
  Genomics   : NCBI Entrez (gene/nuccore/protein/sra/geo/dbsnp/clinvar/…), Ensembl
  Structure  : UniProt, RCSB PDB, AlphaFold DB
  Chemistry  : PubChem, ChEMBL (+ bioactivity)
  Pathways   : KEGG, Reactome, STRING

Usage inside a kernel cell:
    import scidb
    scidb.catalog()                       # list everything
    hits = scidb.lit_search("KRAS G12C covalent inhibitor", limit=8)
    up   = scidb.uniprot("P01116")        # KRAS entry + features
    pc   = scidb.pubchem(name="sotorasib")
    pdb  = scidb.pdb("6OIM")
"""
from __future__ import annotations
import json as _json
import os as _os
import time as _time
import hashlib as _hashlib
import urllib.parse as _uparse
import urllib.request as _urequest
import urllib.error as _uerror

__all__ = [
    "catalog", "lit_search", "pubmed", "crossref", "semantic_scholar",
    "entrez_search", "entrez_fetch", "entrez_summary", "gene", "geo", "ensembl",
    "uniprot", "uniprot_search", "pdb", "pdb_search", "alphafold",
    "pubchem", "chembl", "chembl_activity", "kegg", "reactome", "string_db",
    "opentargets", "clinvar", "gnomad", "gwas_catalog", "clinical_trials", "blast",
    "interpro", "protein_atlas", "rest",
]

_UA = "Axiom3-scidb/1.0 (scientific research; https://axiom3.local)"
_TIMEOUT = 30
_MIN_INTERVAL = 0.34          # be polite: <=3 req/s to any host (NCBI's cap w/o key)
_last_call = {}


def _cache_dir():
    d = _os.path.join(_os.path.expanduser("~"), ".axiom3", "cache", "scidb")
    try:
        _os.makedirs(d, exist_ok=True)
    except Exception:
        return None
    return d


def _throttle(host):
    now = _time.time()
    last = _last_call.get(host, 0.0)
    wait = _MIN_INTERVAL - (now - last)
    if wait > 0:
        _time.sleep(wait)
    _last_call[host] = _time.time()


def _http(url, params=None, is_json=True, headers=None, data=None, cache=True):
    """GET (or POST if data) with retries, polite throttling, and a 24h disk cache."""
    if params:
        url = url + ("&" if "?" in url else "?") + _uparse.urlencode(params)
    key = None
    cd = _cache_dir()
    if cache and cd and data is None:
        key = _os.path.join(cd, _hashlib.sha256(url.encode()).hexdigest()[:32] + (".json" if is_json else ".txt"))
        if _os.path.exists(key) and (_time.time() - _os.path.getmtime(key)) < 86400:
            try:
                raw = open(key, "r", encoding="utf-8").read()
                return _json.loads(raw) if is_json else raw
            except Exception:
                pass
    host = _uparse.urlparse(url).netloc
    hdrs = {"User-Agent": _UA, "Accept": "application/json" if is_json else "*/*"}
    if headers:
        hdrs.update(headers)
    last_err = None
    for attempt in range(3):
        _throttle(host)
        try:
            body = data.encode() if isinstance(data, str) else data
            req = _urequest.Request(url, data=body, headers=hdrs, method="POST" if data is not None else "GET")
            with _urequest.urlopen(req, timeout=_TIMEOUT) as r:
                raw = r.read().decode("utf-8", "replace")
            out = _json.loads(raw) if is_json else raw
            if key:
                try:
                    open(key, "w", encoding="utf-8").write(raw)
                except Exception:
                    pass
            return out
        except _uerror.HTTPError as e:
            last_err = "HTTP %s for %s" % (e.code, url)
            if e.code in (429, 500, 502, 503):
                _time.sleep(1.5 * (attempt + 1))
                continue
            break
        except Exception as e:  # noqa
            last_err = "%s: %s" % (type(e).__name__, e)
            _time.sleep(1.0 * (attempt + 1))
    return {"error": last_err or "request failed", "url": url}


# ────────────────────────────── literature ──────────────────────────────
def lit_search(query, limit=10, source="europepmc"):
    """Search the literature. source='europepmc' (default) | 'crossref' | 'semanticscholar' | 'pubmed'.
    Returns [{title, authors, year, journal, doi, pmid, pmcid, url, source_db}]."""
    if source == "crossref":
        return crossref(query=query, rows=limit)
    if source == "semanticscholar":
        return semantic_scholar(query, limit=limit)
    if source == "pubmed":
        return pubmed(query, retmax=limit)
    j = _http("https://www.ebi.ac.uk/europepmc/webservices/rest/search",
              {"query": query, "format": "json", "pageSize": min(int(limit), 100), "resultType": "core"})
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for r in (j.get("resultList", {}) or {}).get("result", []) or []:
        pmid = r.get("pmid")
        out.append({
            "title": r.get("title"), "authors": r.get("authorString"),
            "year": r.get("pubYear"), "journal": r.get("journalTitle"),
            "doi": r.get("doi"), "pmid": pmid, "pmcid": r.get("pmcid"),
            "citations": r.get("citedByCount"), "is_open_access": r.get("isOpenAccess") == "Y",
            "abstract": r.get("abstractText"),
            "url": ("https://doi.org/" + r["doi"]) if r.get("doi") else
                   ("https://pubmed.ncbi.nlm.nih.gov/%s/" % pmid if pmid else None),
            "source_db": "Europe PMC",
        })
    return out


def pubmed(query, retmax=20):
    """PubMed via NCBI E-utilities (esearch → esummary). Returns citable records."""
    s = entrez_search("pubmed", query, retmax=retmax)
    if isinstance(s, dict) and s.get("error"):
        return s
    ids = s.get("idlist", [])
    if not ids:
        return []
    summ = entrez_summary("pubmed", ids)
    if isinstance(summ, dict) and summ.get("error"):
        return summ
    res = summ.get("result", {})
    out = []
    for uid in res.get("uids", []):
        r = res.get(uid, {})
        doi = next((x.get("value") for x in r.get("articleids", []) if x.get("idtype") == "doi"), None)
        out.append({
            "title": r.get("title"), "authors": ", ".join(a.get("name", "") for a in r.get("authors", [])),
            "year": (r.get("pubdate", "") or "")[:4], "journal": r.get("fulljournalname") or r.get("source"),
            "doi": doi, "pmid": uid,
            "url": "https://pubmed.ncbi.nlm.nih.gov/%s/" % uid, "source_db": "PubMed",
        })
    return out


def crossref(query=None, doi=None, rows=10):
    """Crossref works — by free-text query or a specific DOI."""
    if doi:
        j = _http("https://api.crossref.org/works/" + _uparse.quote(doi))
        if isinstance(j, dict) and j.get("error"):
            return j
        return _crossref_item(j.get("message", {}))
    j = _http("https://api.crossref.org/works", {"query": query, "rows": int(rows)})
    if isinstance(j, dict) and j.get("error"):
        return j
    return [_crossref_item(it) for it in (j.get("message", {}) or {}).get("items", [])]


def _crossref_item(it):
    doi = it.get("DOI")
    return {
        "title": (it.get("title") or [None])[0],
        "authors": ", ".join(("%s %s" % (a.get("given", ""), a.get("family", ""))).strip() for a in it.get("author", []) or []),
        "year": ((it.get("issued", {}) or {}).get("date-parts", [[None]]) or [[None]])[0][0],
        "journal": (it.get("container-title") or [None])[0], "doi": doi,
        "citations": it.get("is-referenced-by-count"),
        "url": "https://doi.org/" + doi if doi else it.get("URL"), "source_db": "Crossref",
    }


def semantic_scholar(query, limit=10):
    """Semantic Scholar paper search (title/abstract/tldr + citation counts)."""
    j = _http("https://api.semanticscholar.org/graph/v1/paper/search",
              {"query": query, "limit": min(int(limit), 100),
               "fields": "title,authors,year,venue,externalIds,citationCount,abstract,tldr"})
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for p in j.get("data", []) or []:
        ext = p.get("externalIds", {}) or {}
        out.append({
            "title": p.get("title"), "authors": ", ".join(a.get("name", "") for a in p.get("authors", []) or []),
            "year": p.get("year"), "journal": p.get("venue"), "doi": ext.get("DOI"),
            "pmid": ext.get("PubMed"), "citations": p.get("citationCount"),
            "tldr": (p.get("tldr") or {}).get("text"), "abstract": p.get("abstract"),
            "url": "https://doi.org/" + ext["DOI"] if ext.get("DOI") else None, "source_db": "Semantic Scholar",
        })
    return out


# ────────────────────────────── NCBI Entrez ──────────────────────────────
_EUTILS = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/"


def entrez_search(db, term, retmax=20):
    """Generic Entrez esearch — db is ANY of the ~40 NCBI databases: pubmed, gene,
    protein, nuccore, nucleotide, sra, gds (GEO), dbsnp, clinvar, assembly, genome,
    taxonomy, biosample, bioproject, pmc, mesh, structure, … Returns {count, idlist}."""
    j = _http(_EUTILS + "esearch.fcgi",
              {"db": db, "term": term, "retmax": int(retmax), "retmode": "json"})
    if isinstance(j, dict) and j.get("error"):
        return j
    r = j.get("esearchresult", {})
    return {"count": r.get("count"), "idlist": r.get("idlist", []), "db": db,
            "source_url": _EUTILS + "esearch.fcgi?db=%s&term=%s" % (db, _uparse.quote(str(term)))}


def entrez_summary(db, ids):
    ids = ",".join(map(str, ids)) if isinstance(ids, (list, tuple)) else str(ids)
    return _http(_EUTILS + "esummary.fcgi", {"db": db, "id": ids, "retmode": "json"})


def entrez_fetch(db, ids, rettype="gb", retmode="text"):
    """Fetch full records (e.g. db='nuccore', rettype='gb'/'fasta'; db='protein', rettype='fasta')."""
    ids = ",".join(map(str, ids)) if isinstance(ids, (list, tuple)) else str(ids)
    return _http(_EUTILS + "efetch.fcgi", {"db": db, "id": ids, "rettype": rettype, "retmode": retmode},
                 is_json=(retmode == "json"))


def gene(symbol, organism="Homo sapiens"):
    """Look up a gene by symbol → NCBI Gene summary (official name, aliases, location, summary)."""
    s = entrez_search("gene", "%s[sym] AND %s[orgn]" % (symbol, organism), retmax=1)
    if isinstance(s, dict) and s.get("error"):
        return s
    if not s.get("idlist"):
        return {"error": "no gene found for %s in %s" % (symbol, organism)}
    uid = s["idlist"][0]
    summ = entrez_summary("gene", uid)
    r = (summ.get("result", {}) or {}).get(uid, {}) if isinstance(summ, dict) else {}
    return {
        "gene_id": uid, "symbol": r.get("name"), "description": r.get("description"),
        "aliases": r.get("otheraliases"), "summary": r.get("summary"),
        "chromosome": r.get("chromosome"), "map_location": r.get("maplocation"),
        "organism": (r.get("organism", {}) or {}).get("scientificname"),
        "url": "https://www.ncbi.nlm.nih.gov/gene/%s" % uid, "source_db": "NCBI Gene",
    }


def geo(query, retmax=10):
    """Search GEO (Gene Expression Omnibus) series/datasets via Entrez 'gds'."""
    s = entrez_search("gds", query, retmax=retmax)
    if isinstance(s, dict) and s.get("error"):
        return s
    if not s.get("idlist"):
        return []
    summ = entrez_summary("gds", s["idlist"])
    res = summ.get("result", {}) if isinstance(summ, dict) else {}
    out = []
    for uid in res.get("uids", []):
        r = res.get(uid, {})
        acc = r.get("accession")
        out.append({"accession": acc, "title": r.get("title"), "summary": r.get("summary"),
                    "organism": r.get("taxon"), "n_samples": r.get("n_samples"), "gdstype": r.get("gdstype"),
                    "url": "https://www.ncbi.nlm.nih.gov/geo/query/acc.cgi?acc=%s" % acc if acc else None,
                    "source_db": "GEO"})
    return out


def ensembl(symbol_or_id, species="human", kind="lookup"):
    """Ensembl REST. kind='lookup' (by stable id or symbol) → gene coords/biotype/description."""
    if str(symbol_or_id).upper().startswith("ENS"):
        url = "https://rest.ensembl.org/lookup/id/%s" % symbol_or_id
    else:
        url = "https://rest.ensembl.org/lookup/symbol/%s/%s" % (species, symbol_or_id)
    j = _http(url, {"content-type": "application/json", "expand": 0})
    if isinstance(j, dict) and not j.get("error"):
        j["url"] = "https://www.ensembl.org/%s/Gene/Summary?g=%s" % (species, j.get("id", ""))
        j["source_db"] = "Ensembl"
    return j


# ────────────────────────────── protein / structure ──────────────────────────────
def uniprot(accession):
    """UniProtKB entry by accession (e.g. 'P01116') → name, gene, function, sequence, features."""
    j = _http("https://rest.uniprot.org/uniprotkb/%s.json" % accession)
    if isinstance(j, dict) and j.get("error"):
        return j
    feats = [{"type": f.get("type"),
              "start": (f.get("location", {}).get("start", {}) or {}).get("value"),
              "end": (f.get("location", {}).get("end", {}) or {}).get("value"),
              "description": f.get("description")} for f in j.get("features", []) or []]
    fn = ""
    for c in j.get("comments", []) or []:
        if c.get("commentType") == "FUNCTION":
            fn = " ".join(t.get("value", "") for t in c.get("texts", []) or [])
            break
    return {
        "accession": j.get("primaryAccession"),
        "id": j.get("uniProtkbId"),
        "protein_name": ((j.get("proteinDescription", {}) or {}).get("recommendedName", {}) or {}).get("fullName", {}).get("value"),
        "gene": (j.get("genes", [{}]) or [{}])[0].get("geneName", {}).get("value") if j.get("genes") else None,
        "organism": (j.get("organism", {}) or {}).get("scientificName"),
        "length": (j.get("sequence", {}) or {}).get("length"),
        "sequence": (j.get("sequence", {}) or {}).get("value"),
        "function": fn, "features": feats,
        "pdb_xrefs": [x.get("id") for x in j.get("uniProtKBCrossReferences", []) or [] if x.get("database") == "PDB"],
        "url": "https://www.uniprot.org/uniprotkb/%s" % j.get("primaryAccession", accession), "source_db": "UniProt",
    }


def uniprot_search(query, limit=10):
    j = _http("https://rest.uniprot.org/uniprotkb/search",
              {"query": query, "format": "json", "size": min(int(limit), 100),
               "fields": "accession,id,protein_name,gene_names,organism_name,length"})
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for r in j.get("results", []) or []:
        acc = r.get("primaryAccession")
        out.append({"accession": acc, "id": r.get("uniProtkbId"),
                    "protein_name": ((r.get("proteinDescription", {}) or {}).get("recommendedName", {}) or {}).get("fullName", {}).get("value"),
                    "organism": (r.get("organism", {}) or {}).get("scientificName"),
                    "url": "https://www.uniprot.org/uniprotkb/%s" % acc, "source_db": "UniProt"})
    return out


def pdb(pdb_id):
    """RCSB PDB entry metadata (title, method, resolution, organism, ligands, citation)."""
    pid = str(pdb_id).upper()
    j = _http("https://data.rcsb.org/rest/v1/core/entry/%s" % pid)
    if isinstance(j, dict) and j.get("error"):
        return j
    cit = (j.get("rcsb_primary_citation", {}) or {})
    return {
        "pdb_id": pid, "title": (j.get("struct", {}) or {}).get("title"),
        "method": (j.get("exptl", [{}]) or [{}])[0].get("method"),
        "resolution": (j.get("rcsb_entry_info", {}) or {}).get("resolution_combined", [None])[0],
        "deposited": (j.get("rcsb_accession_info", {}) or {}).get("deposit_date"),
        "doi": cit.get("pdbx_database_id_doi"), "pubmed": cit.get("pdbx_database_id_pub_med"),
        "n_polymer_entities": (j.get("rcsb_entry_info", {}) or {}).get("polymer_entity_count"),
        "url": "https://www.rcsb.org/structure/%s" % pid,
        "coords_url": "https://files.rcsb.org/download/%s.pdb" % pid, "source_db": "RCSB PDB",
    }


def pdb_search(query, limit=10):
    """Full-text search RCSB PDB → list of pdb ids (feed each to pdb())."""
    q = {"query": {"type": "terminal", "service": "full_text", "parameters": {"value": query}},
         "return_type": "entry",
         "request_options": {"paginate": {"start": 0, "rows": int(limit)}}}
    j = _http("https://search.rcsb.org/rcsbsearch/v2/query", {"json": _json.dumps(q)})
    if isinstance(j, dict) and j.get("error"):
        return j
    return [{"pdb_id": r.get("identifier"), "score": r.get("score"),
             "url": "https://www.rcsb.org/structure/%s" % r.get("identifier"), "source_db": "RCSB PDB"}
            for r in j.get("result_set", []) or []]


def alphafold(uniprot_acc):
    """AlphaFold DB predicted model for a UniProt accession (pLDDT, model + PAE urls)."""
    j = _http("https://alphafold.ebi.ac.uk/api/prediction/%s" % uniprot_acc)
    if isinstance(j, dict) and j.get("error"):
        return j
    if not j:
        return {"error": "no AlphaFold model for %s" % uniprot_acc}
    m = j[0]
    return {"uniprot": m.get("uniprotAccession"), "gene": m.get("gene"), "organism": m.get("organismScientificName"),
            "model_url": m.get("pdbUrl"), "cif_url": m.get("cifUrl"), "pae_url": m.get("paeImageUrl"),
            "mean_plddt": m.get("globalMetricValue"),
            "note": "PREDICTED model — not an experimental structure; colour by pLDDT and label as such.",
            "url": "https://alphafold.ebi.ac.uk/entry/%s" % uniprot_acc, "source_db": "AlphaFold DB"}


# ────────────────────────────── chemistry ──────────────────────────────
_PC_PROPS = "MolecularFormula,MolecularWeight,CanonicalSMILES,IsomericSMILES,InChIKey,IUPACName,XLogP,TPSA,HBondDonorCount,HBondAcceptorCount,RotatableBondCount"


def pubchem(name=None, cid=None, smiles=None, inchikey=None):
    """PubChem compound → computed properties (MW, formula, SMILES, InChIKey, XLogP, TPSA, …)."""
    if cid is not None:
        ns = "cid/%s" % cid
    elif smiles:
        ns = "smiles/%s" % _uparse.quote(smiles)
    elif inchikey:
        ns = "inchikey/%s" % inchikey
    elif name:
        ns = "name/%s" % _uparse.quote(name)
    else:
        return {"error": "need one of name/cid/smiles/inchikey"}
    j = _http("https://pubchem.ncbi.nlm.nih.gov/rest/pug/compound/%s/property/%s/JSON" % (ns, _PC_PROPS))
    if isinstance(j, dict) and j.get("error"):
        return j
    props = (j.get("PropertyTable", {}) or {}).get("Properties", [])
    if not props:
        return {"error": "no PubChem compound for %s" % (name or cid or smiles or inchikey)}
    p = props[0]
    p["url"] = "https://pubchem.ncbi.nlm.nih.gov/compound/%s" % p.get("CID")
    p["source_db"] = "PubChem"
    p["_note"] = "descriptors are COMPUTED (label as calc.)"
    return p


def chembl(query=None, chembl_id=None, limit=10):
    """ChEMBL molecule — by ChEMBL id or free-text search (name/synonym)."""
    if chembl_id:
        j = _http("https://www.ebi.ac.uk/chembl/api/data/molecule/%s.json" % chembl_id)
        if isinstance(j, dict) and j.get("error"):
            return j
        return _chembl_mol(j)
    j = _http("https://www.ebi.ac.uk/chembl/api/data/molecule/search",
              {"q": query, "format": "json", "limit": int(limit)})
    if isinstance(j, dict) and j.get("error"):
        return j
    return [_chembl_mol(m) for m in j.get("molecules", []) or []]


def _chembl_mol(m):
    props = m.get("molecule_properties", {}) or {}
    struct = m.get("molecule_structures", {}) or {}
    cid = m.get("molecule_chembl_id")
    return {
        "chembl_id": cid, "name": m.get("pref_name"), "max_phase": m.get("max_phase"),
        "molecular_weight": props.get("full_mwt"), "alogp": props.get("alogp"),
        "psa": props.get("psa"), "hbd": props.get("hbd"), "hba": props.get("hba"),
        "canonical_smiles": struct.get("canonical_smiles"), "inchikey": struct.get("standard_inchi_key"),
        "url": "https://www.ebi.ac.uk/chembl/compound_report_card/%s/" % cid if cid else None, "source_db": "ChEMBL",
    }


def chembl_activity(target_chembl_id=None, molecule_chembl_id=None, limit=50):
    """ChEMBL measured bioactivities (IC50/Ki/EC50) for a target or a molecule — REAL assay values, cite them."""
    params = {"format": "json", "limit": int(limit)}
    if target_chembl_id:
        params["target_chembl_id"] = target_chembl_id
    if molecule_chembl_id:
        params["molecule_chembl_id"] = molecule_chembl_id
    if len(params) == 2:
        return {"error": "need target_chembl_id or molecule_chembl_id"}
    j = _http("https://www.ebi.ac.uk/chembl/api/data/activity", params)
    if isinstance(j, dict) and j.get("error"):
        return j
    return [{"molecule_chembl_id": a.get("molecule_chembl_id"), "target_chembl_id": a.get("target_chembl_id"),
             "type": a.get("standard_type"), "relation": a.get("standard_relation"),
             "value": a.get("standard_value"), "units": a.get("standard_units"),
             "assay": a.get("assay_description"), "doc_doi": a.get("document_journal"),
             "source_db": "ChEMBL"} for a in j.get("activities", []) or []]


# ────────────────────────────── pathways / interactions ──────────────────────────────
def kegg(entry):
    """KEGG record by id (e.g. 'hsa04010' pathway, 'K04408' KO, 'C00031' compound, 'hsa:3845' gene). Raw flat text."""
    txt = _http("https://rest.kegg.jp/get/%s" % entry, is_json=False)
    if isinstance(txt, dict) and txt.get("error"):
        return txt
    return {"entry": entry, "text": txt, "url": "https://www.kegg.jp/entry/%s" % entry, "source_db": "KEGG"}


def reactome(query, species="Homo sapiens", limit=10):
    """Reactome pathway search (ContentService)."""
    j = _http("https://reactome.org/ContentService/search/query",
              {"query": query, "species": species, "cluster": "true"})
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for grp in j.get("results", []) or []:
        for e in grp.get("entries", [])[:limit]:
            sid = e.get("stId")
            out.append({"stId": sid, "name": e.get("name"), "type": e.get("exactType"), "species": e.get("species"),
                        "url": "https://reactome.org/content/detail/%s" % sid if sid else None, "source_db": "Reactome"})
    return out


def string_db(identifiers, species=9606, required_score=400):
    """STRING protein–protein interaction network. identifiers=list of gene symbols; species NCBI taxid (9606=human)."""
    ids = "%0d".join(identifiers) if isinstance(identifiers, (list, tuple)) else str(identifiers)
    j = _http("https://string-db.org/api/json/network",
              {"identifiers": ids, "species": int(species), "required_score": int(required_score),
               "caller_identity": "axiom3"})
    if isinstance(j, dict) and j.get("error"):
        return j
    return [{"a": e.get("preferredName_A"), "b": e.get("preferredName_B"), "score": e.get("score"),
             "source_db": "STRING"} for e in j] if isinstance(j, list) else j


# ──────────────── drug targets / clinical / variants (P0 expansion) ────────────────
def opentargets(query=None, ensembl_id=None, limit=10):
    """Open Targets Platform — gene→disease associations for drug-target discovery / repurposing.
    Pass a gene SYMBOL (query, e.g. 'EGFR') or an Ensembl gene id (ensembl_id). Returns the target's
    top associated diseases with an evidence-integrated association score (0–1). Cite the source_url."""
    api = "https://api.platform.opentargets.org/api/v4/graphql"

    def _gql(q):
        return _http(api, data=_json.dumps({"query": q}), headers={"Content-Type": "application/json"}, cache=True)
    tid = ensembl_id
    if not tid and query:
        s = _gql('{search(queryString:"%s",entityNames:["target"]){hits{id name}}}' % str(query).replace('"', ""))
        if isinstance(s, dict) and s.get("error"):
            return s
        hits = (((s or {}).get("data") or {}).get("search") or {}).get("hits") or []
        if not hits:
            return {"error": "no Open Targets target for '%s'" % query, "source_db": "OpenTargets"}
        tid = hits[0].get("id")
    if not tid:
        return {"error": "opentargets needs query (gene symbol) or ensembl_id", "source_db": "OpenTargets"}
    q = ('{target(ensemblId:"%s"){approvedSymbol approvedName '
         'associatedDiseases(page:{index:0,size:%d}){count rows{score disease{id name}}}}}' % (tid, int(limit)))
    j = _gql(q)
    if isinstance(j, dict) and j.get("error"):
        return j
    t = (((j or {}).get("data") or {}).get("target")) or {}
    if not t:
        return {"error": "no Open Targets data for %s" % tid, "source_db": "OpenTargets"}
    ad = (t.get("associatedDiseases") or {})
    diseases = [{"disease": (r.get("disease") or {}).get("name"), "efo_id": (r.get("disease") or {}).get("id"),
                 "score": round(r.get("score", 0.0), 4)} for r in (ad.get("rows") or [])]
    return {"ensembl_id": tid, "symbol": t.get("approvedSymbol"), "name": t.get("approvedName"),
            "disease_count": ad.get("count"), "associated_diseases": diseases,
            "url": "https://platform.opentargets.org/target/%s" % tid, "source_db": "OpenTargets"}


def clinvar(query, retmax=10):
    """ClinVar — clinical significance of genetic variants. Query e.g. 'BRCA1 pathogenic', 'EGFR T790M',
    an HGVS name, or an rsID. Returns variants with clinical significance + gene + condition + review status."""
    es = entrez_search("clinvar", query, retmax)
    if isinstance(es, dict) and es.get("error"):
        return es
    ids = (es or {}).get("idlist") or []
    if not ids:
        return {"variants": [], "note": "no ClinVar records for '%s'" % query, "source_db": "ClinVar"}
    su = entrez_summary("clinvar", ",".join(ids))
    if isinstance(su, dict) and su.get("error"):
        return su
    res = ((su or {}).get("result") or {})
    out = []
    for uid in res.get("uids", []) or []:
        r = res.get(uid, {}) or {}
        germ = r.get("germline_classification") or {}
        sig = germ.get("description") or ((r.get("clinical_significance") or {}) or {}).get("description")
        conds = ";".join(t.get("trait_name", "") for t in (germ.get("trait_set") or [])) or \
            ";".join(t.get("trait_name", "") for ts in (r.get("trait_set") or []) for t in [ts])
        out.append({"uid": uid, "title": r.get("title"),
                    "clinical_significance": sig, "review_status": germ.get("review_status"),
                    "gene": ";".join(g.get("symbol", "") for g in (r.get("genes") or [])),
                    "conditions": conds,
                    "url": "https://www.ncbi.nlm.nih.gov/clinvar/variation/%s/" % uid, "source_db": "ClinVar"})
    return {"variants": out, "source_db": "ClinVar"}


def gnomad(variant_id, dataset="gnomad_r4"):
    """gnomAD population allele frequencies. variant_id in GRCh38 'chrom-pos-ref-alt' form (e.g.
    '1-55051215-G-A'). Returns global + per-population allele frequency — use it to judge whether a
    variant is rare enough to be plausibly pathogenic (a common variant argues AGAINST pathogenicity)."""
    q = ('query V($id:String!,$ds:DatasetId!){variant(variantId:$id,dataset:$ds){variant_id rsids '
         'genome{ac an af homozygote_count populations{id ac an}} exome{ac an af}}}')
    j = _http("https://gnomad.broadinstitute.org/api",
              data=_json.dumps({"query": q, "variables": {"id": variant_id, "ds": dataset}}),
              headers={"Content-Type": "application/json"}, cache=True)
    if isinstance(j, dict) and j.get("error"):
        return j
    v = (((j or {}).get("data") or {}).get("variant"))
    if not v:
        return {"error": "no gnomAD record for %s (need GRCh38 chrom-pos-ref-alt)" % variant_id, "source_db": "gnomAD"}
    g = v.get("genome") or {}
    pops = [{"pop": p.get("id"), "af": (p.get("ac") / p.get("an")) if p.get("an") else None,
             "ac": p.get("ac"), "an": p.get("an")} for p in (g.get("populations") or []) if "_" not in (p.get("id") or "_")]
    return {"variant_id": v.get("variant_id"), "rsids": v.get("rsids"),
            "genome_af": g.get("af"), "genome_ac": g.get("ac"), "genome_an": g.get("an"),
            "homozygotes": g.get("homozygote_count"), "exome_af": (v.get("exome") or {}).get("af"),
            "populations": pops,
            "url": "https://gnomad.broadinstitute.org/variant/%s?dataset=%s" % (variant_id, dataset), "source_db": "gnomAD"}


def gwas_catalog(gene, limit=15):
    """GWAS Catalog — published genome-wide association SNPs mapped to a GENE (e.g. 'TCF7L2', 'EGFR').
    Returns the gene's GWAS-associated SNPs (rsID, functional class, chromosome/position). A hit means
    that variant reached genome-wide significance for some trait in a published GWAS."""
    j = _http("https://www.ebi.ac.uk/gwas/rest/api/singleNucleotidePolymorphisms/search/findByGene",
              {"geneName": str(gene)})
    if isinstance(j, dict) and j.get("error"):
        return j
    snps = ((j.get("_embedded") or {}) or {}).get("singleNucleotidePolymorphisms") or []
    out = []
    for s in snps[: int(limit)]:
        loc = (s.get("locations") or [{}])[0] if s.get("locations") else {}
        out.append({"rsid": s.get("rsId"), "functional_class": s.get("functionalClass"),
                    "chromosome": loc.get("chromosomeName"), "position": loc.get("chromosomePosition"),
                    "url": "https://www.ebi.ac.uk/gwas/variants/%s" % s.get("rsId"), "source_db": "GWAS Catalog"})
    return {"gene": gene, "snp_count": len(snps), "snps": out, "source_db": "GWAS Catalog"}


def clinical_trials(query, limit=10):
    """ClinicalTrials.gov (API v2) — registered clinical trials. Query a condition, drug, or intervention
    (e.g. 'EGFR NSCLC osimertinib'). Returns trials: NCT id, title, phase, status, conditions, interventions."""
    j = _http("https://clinicaltrials.gov/api/v2/studies",
              {"query.term": str(query), "pageSize": int(limit), "format": "json"})
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for s in (j.get("studies") or []):
        p = s.get("protocolSection") or {}
        idm = p.get("identificationModule") or {}
        st = p.get("statusModule") or {}
        dm = p.get("designModule") or {}
        cm = p.get("conditionsModule") or {}
        im = p.get("armsInterventionsModule") or {}
        nct = idm.get("nctId")
        out.append({"nct_id": nct, "title": idm.get("briefTitle"),
                    "phase": ";".join(dm.get("phases") or []), "status": st.get("overallStatus"),
                    "conditions": cm.get("conditions") or [],
                    "interventions": [i.get("name") for i in (im.get("interventions") or [])],
                    "url": "https://clinicaltrials.gov/study/%s" % nct if nct else None, "source_db": "ClinicalTrials.gov"})
    return {"trials": out, "source_db": "ClinicalTrials.gov"}


def blast(sequence, program="blastp", database="swissprot", hitlist_size=10, max_wait=90):
    """NCBI BLAST sequence-similarity search. sequence = a protein or nucleotide sequence (raw or FASTA).
    program = blastp | blastn | blastx | tblastn. database = swissprot | nr | nt | refseq_protein.
    Submits + polls (BLAST is SLOW — waits up to max_wait s, then returns the RID so you can retry).
    Returns top hits (accession, description, %identity, e-value)."""
    import re as _re
    base = "https://blast.ncbi.nlm.nih.gov/Blast.cgi"
    seq = str(sequence).strip()
    put = _http(base, is_json=False, cache=False,
                data=_uparse.urlencode({"CMD": "Put", "PROGRAM": program, "DATABASE": database,
                                        "QUERY": seq, "HITLIST_SIZE": int(hitlist_size)}),
                headers={"Content-Type": "application/x-www-form-urlencoded"})
    if isinstance(put, dict) and put.get("error"):
        return put
    m = _re.search(r"RID = (\S+)", put or "")
    if not m:
        return {"error": "BLAST submit failed (no RID)", "source_db": "BLAST"}
    rid = m.group(1)
    waited = 0
    ready = False
    while waited < int(max_wait):
        _time.sleep(10)
        waited += 10
        stat = _http(base, {"CMD": "Get", "FORMAT_OBJECT": "SearchInfo", "RID": rid}, is_json=False, cache=False)
        if isinstance(stat, str) and "Status=READY" in stat:
            ready = True
            break
        if isinstance(stat, str) and "Status=FAILED" in stat:
            return {"error": "BLAST search failed", "rid": rid, "source_db": "BLAST"}
    if not ready:
        return {"status": "running", "rid": rid,
                "note": "BLAST %ss 内未完成;稍后 scidb.blast(...) 会命中缓存或用 CMD=Get&RID=%s 再取" % (max_wait, rid),
                "source_db": "BLAST"}
    res = _http(base, {"CMD": "Get", "FORMAT_TYPE": "JSON2_S", "RID": rid}, is_json=True, cache=False)
    if isinstance(res, dict) and res.get("error"):
        return res
    try:
        search = res["BlastOutput2"][0]["report"]["results"]["search"]
        hits = []
        for h in (search.get("hits") or [])[: int(hitlist_size)]:
            desc = (h.get("description") or [{}])[0]
            hs = (h.get("hsps") or [{}])[0]
            ident = hs.get("identity")
            alen = hs.get("align_len")
            hits.append({"accession": desc.get("accession"), "title": desc.get("title"),
                         "evalue": hs.get("evalue"),
                         "percent_identity": round(100.0 * ident / alen, 1) if ident and alen else None,
                         "bit_score": hs.get("bit_score"), "source_db": "BLAST(%s)" % database})
        return {"program": program, "database": database, "rid": rid, "hits": hits, "source_db": "BLAST"}
    except Exception as e:  # noqa
        return {"error": "BLAST result parse failed: %s" % e, "rid": rid, "source_db": "BLAST"}


def interpro(uniprot_acc):
    """InterPro protein DOMAIN architecture for a UniProt accession (e.g. 'P01116') — the canonical
    domains / families / sites (Pfam, PROSITE, SMART … integrated). Use it to describe a protein's
    functional modules, beyond UniProt's own feature list."""
    j = _http("https://www.ebi.ac.uk/interpro/api/entry/interpro/protein/uniprot/%s" % uniprot_acc)
    if isinstance(j, dict) and j.get("error"):
        return j
    out = []
    for e in j.get("results", []) or []:
        md = e.get("metadata", {}) or {}
        out.append({"accession": md.get("accession"), "type": md.get("type"), "name": md.get("name"),
                    "url": "https://www.ebi.ac.uk/interpro/entry/interpro/%s/" % md.get("accession"), "source_db": "InterPro"})
    return {"uniprot": uniprot_acc, "entries": out, "source_db": "InterPro"}


def protein_atlas(ensembl_id):
    """Human Protein Atlas expression + localization for an ENSEMBL GENE id (e.g. 'ENSG00000133703';
    resolve a symbol→Ensembl via scidb.ensembl or scidb.opentargets). Returns RNA tissue specificity,
    subcellular location, and biological-process annotation — where a protein is expressed/localized
    (biomarker / target context)."""
    j = _http("https://www.proteinatlas.org/%s.json" % ensembl_id)
    if isinstance(j, dict) and j.get("error"):
        return j
    if not isinstance(j, dict):
        return {"error": "unexpected HPA response", "source_db": "HPA"}
    return {"gene": j.get("Gene"), "ensembl": ensembl_id,
            "rna_tissue_specificity": j.get("RNA tissue specificity"),
            "subcellular_location": j.get("Subcellular location"),
            "biological_process": j.get("Biological process"),
            "url": "https://www.proteinatlas.org/%s" % ensembl_id, "source_db": "Human Protein Atlas"}


def rest(url, params=None, headers=None, is_json=True):
    """Generic provenance-tracked fetch for ANY public REST database/API that scidb has no dedicated
    function for (e.g. a niche registry, a species-specific resource). Reuses the same polite throttle +
    24h cache + retries as every other scidb call. Returns {'source_url', 'data'} — cite source_url as
    [Sn]. Only public read-only http(s) endpoints; do NOT put credentials/tokens in the URL."""
    r = _http(url, params=params, headers=headers, is_json=is_json)
    if isinstance(r, dict) and r.get("error"):
        return r
    src = url + (("?" + _uparse.urlencode(params)) if params else "")
    return {"source_url": src, "data": r}


# ────────────────────────────── catalog ──────────────────────────────
_CATALOG = {
    "literature": ["lit_search(query, source=europepmc|crossref|semanticscholar|pubmed)", "pubmed(query)", "crossref(query|doi)", "semantic_scholar(query)"],
    "genomics (NCBI Entrez ~40 dbs)": ["entrez_search(db, term)", "entrez_fetch(db, ids, rettype)", "entrez_summary(db, ids)", "gene(symbol, organism)", "geo(query)", "ensembl(symbol_or_id, species)"],
    "protein / structure": ["uniprot(acc)", "uniprot_search(query)", "pdb(pdb_id)", "pdb_search(query)", "alphafold(uniprot_acc)", "interpro(uniprot_acc)  # domains", "protein_atlas(ensembl_id)  # expression/localization"],
    "chemistry": ["pubchem(name|cid|smiles|inchikey)", "chembl(query|chembl_id)", "chembl_activity(target_chembl_id|molecule_chembl_id)"],
    "pathways / interactions": ["kegg(entry)", "reactome(query)", "string_db(identifiers, species)"],
    "drug targets / disease": ["opentargets(query=gene | ensembl_id)  # target→disease assoc scores (repurposing)"],
    "clinical variants / genetics": ["clinvar(query)  # variant clinical significance", "gnomad(variant_id)  # population allele freq", "gwas_catalog(gene)  # GWAS SNPs for a gene"],
    "clinical trials": ["clinical_trials(query)  # ClinicalTrials.gov v2"],
    "sequence search": ["blast(sequence, program, database)  # NCBI BLAST (slow, submit+poll)"],
    "generic": ["rest(url, params)  # any other public REST DB, provenance-tracked → {source_url, data}"],
}


def catalog():
    """Print the available databases + call signatures. Every result carries a source_url for [Sn] citation."""
    print("scidb — pre-configured scientific databases (results are real records, cite the source_url):")
    for domain, fns in _CATALOG.items():
        print("\n  [%s]" % domain)
        for f in fns:
            print("    scidb.%s" % f)
    print("\n  Honesty: on failure a function returns {'error': ...}; it never fabricates a record.")
    return list(_CATALOG.keys())
