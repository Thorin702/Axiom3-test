# 私有 DB / 存储层决策(开发前)

> 版本 v1 · 2026-06-29 · 回答:私有 DB 有必要吗?单独建专有 DB 管理器,还是一股脑扔项目目录?
> 承重事实经核验:**5 confirmed / 3 uncertain / 0 refuted**(数字层面更正见附录)。

---

## 0. 结论:两者都要,但都不是你说的那个极端

**这是个伪二选一。正确答案是混合,且大部分已被之前的设计逼出来。**

- **原始素材 → 留作项目目录里的普通文件**(事实源、可移植、可人工查看)。你"扔进项目目录"的直觉**对原始输入是对的**。
- **派生与人工产物 → 进一个受管的嵌入式存储**(放在**同一个项目目录内**,如 `.workbench/`)。因为整个产品就是查关系 + 向量检索 + 防篡改溯源——松散文件做不到。
- **但不是"单独的专有 DB 管理器"**(那意味着独立服务/运维/安装,违背你"单二进制、零运维、无 Node/Python"的 Tauri 路线)。**正确的库是"隐形"的**:编译进 App、无需配置、用户根本感觉不到它存在。

> 一个是档案柜(文件),一个是账本+卡片目录(库)。

---

## 1. 划分只看一条规则:可重建性

> **"删掉之后,电脑能不能从原始文件重建它?"**

- **能 → 一次性缓存(cache)**:从不备份,一条命令重建。
- **不能 → 持久事实源(durable)**:加密、备份、只写不改(write-once)。

这就是事件溯源的格言"events are truth, state is a cache"用到我们的流水线上。

---

## 2. 文件 vs 库 的具体划分

**留作普通文件:**
- 项目目录里用户丢进来的原始源(PDF/数据/代码/音视频),阅读流水线只读、永不拥有。
- **内容寻址归档** `.workbench/cas/`:按 BLAKE3 哈希存源字节(即使原链接 link-rot 或源变了,证据仍可验)。字节存盘、不存库——更易备份/去重/流式,且能扛 DB 损坏。
- 审计/事件日志**额外镜像**为 append-only JSONL(`.workbench/audit/`):git-diff 友好、可 grep、可外部锚定、不依赖 DB 引擎也能读。
- **导出/可移植**:RO-Crate(`ro-crate-metadata.json` + 载荷)、PROV-O RDF、完整性"成绩单"——都以开放格式写进目录。

**进受管存储(DURABLE-PRIMARY,必须备份、不可静默编辑):**
原子 claim;verdict(不可变、版本化);证据片段(外键到源 BLAKE3 哈希 + 偏移);校验者身份(人 或 模型+版本+prompt);反驳尝试;裁决结果;校准历史;人工决定/检查点;源清单表;笔记/asks(记忆层的持久侧)。

**进受管存储(REBUILDABLE-CACHE,从不备份、一键重建):**
解析/TEI/表 JSON/转写、分块、嵌入 + ANN 索引(LanceDB)、FTS5 词法索引、任何图投影。

---

## 3. 具体技术选型(2026,Rust/Tauri/Rig)

**两个嵌入式引擎,不要让一个库两头都做:**

1. **关系系统-of-record:SQLite**(rusqlite,STRICT 表 + JSON1,编译进二进制、无 sidecar)。**静态加密用 SQLCipher**(rusqlite 的 `bundled-sqlcipher-vendored-openssl`,AES-256,~5–15% 开销),密钥取自**你已采用的 OS keychain**(keyring crate)。拆成 `durable.db`(备份)+ `cache.db`(可重建),同引擎、两文件、按可重建性打标。
2. **向量索引:LanceDB 嵌入式**(Rust 原生、Arrow/Lance 列式、on-disk IVF_PQ),经 **Rig 原生 `rig-lancedb`** 接入。~1M 向量 >0.95 召回、<10ms、低内存,属 cache 层(可重建)。
   - **不要用 sqlite-vec 当研究规模的向量库**:它仍是 pre-v1 alpha、纯暴力 KNN、过 ~10 万向量明显退化(1M/192 维 ~192ms、3072 维 多秒)。只适合 App 内几千条的小索引。
3. **claim 图(claim→证据→源→校验者→verdict)用关系建模 + 递归 CTE 遍历**(本规模/深度单位数 ms)。**不要引入嵌入式图引擎**:最可能的候选 KuzuDB 据称已被 Apple 收购并归档(此条核验为 uncertain,但"用关系+CTE"的结论与该传闻无关、独立成立)。
4. **BLAKE3 内容寻址**(你阅读流水线已有)做增量再嵌入(只重算 ~10–15% 变化分块)。
5. **硬锁版本**(rig-core 58 个发布里 38 个 breaking;对齐 arrow-rs 版本;`lzma-sys` 用 `static` feature 以便 Tauri 跨平台打包)。

**服务器选项(机构/共享):Postgres + pgvector**,经 Rig 原生 `rig-postgres`,**同一套逻辑 schema**。把存储层放在一个 Rust trait 后面(`DurableStore` + `VectorStore` + `EventLog`),SQLite↔Postgres、LanceDB↔pgvector 是**改配置不是重写**——和部署三档"切换不分叉"一致。**触发切换**:多写并发 / 跨机共享语料 / 跨机聚合 / 服务端重索引。它**非零运维**(要跑 Postgres),所以是逃生舱、不是桌面默认。

---

## 4. 数据模型要点

- **核心实体**(durable.db,STRICT):`claim` / `source(blake3_hash,uri,mime,…)` / `evidence_span(source_id, blake3_hash, 偏移, verbatim_text, parse_trust)` / `verifier(kind=human|model, model_id, version, prompt_hash)` / `verdict(claim_id, verifier_id, status∈{verified,refuted,uncertain}, calibrated_confidence, revises_verdict_id?)`。证据**钉在不可变源字节+偏移**上,源变了 verdict 仍可验。
- **边/连接表**(claim 图,递归 CTE):`verdict_uses_evidence(stance∈{supports,refutes})` / `claim_relation(type∈{subclaim,supports,contradicts})` / `refuter_attempt` / `adjudication` / `human_decision`。
- **verdict 不可变、版本化**:重判是**插新行 + 事件**,经 `revises_verdict_id` 指向旧版(PROV `wasRevisionOf`)——契合"绝不静默把 uncertain 升为 verified"。
- **防篡改(由简到繁)**:append-only **哈希链事件日志** `event(seq, ts, actor, op, payload_json, prev_hash, entry_hash)`,`entry_hash=H(prev_hash‖canonical(payload))`;当前状态是从日志重放的**投影**(事件溯源,顺带白送崩溃续跑)。**定期把链头哈希外部锚定**(提交 git / 打印 / 邮寄)——哈希链是 tamper-**evident** 非 tamper-**proof**,无外部锚定时有 DB 写权限者能重算整条链。仅当需第三方审计/监管免信任核验时,才上 RFC-6962 Merkle。
- **provenance = W3C PROV-O** 作规范词汇(干净映射你的 `数据→变换→结果→claim→源`):实体/活动/Agent + wasGeneratedBy/used/wasDerivedFrom/…;关系存 SQLite,PROV-O RDF 是**导出投影**(需 SPARQL 时用 Oxigraph,但其 SPARQL 未优化,仅作可选导出层)。

---

## 5. 作用域

- **每项目库(默认)**:每个项目目录一个 `.workbench/`(`durable.db` SQLCipher + `cache.db` + LanceDB 目录 + CAS 字节 + 审计 JSONL)。项目自包含可移植(拷/zip/RO-Crate 整个文件夹,验证过的研究对象整体搬走),一个项目的损坏/密钥与另一个隔离。**密钥按项目**(共享一个项目不暴露其他)。
- **全局/用户库**(`~/.config` 级小 SQLite):跨项目**非证据性**状态——语义偏好、可泛化的校准先验、模型能力矩阵缓存、API 凭据**指针**(真 key 仍只在 OS keychain)、已知项目索引。**口诀:是证据/verdict → 每项目(绝不跨项目泄漏);是用户偏好/可复用校准 → 全局。**

---

## 6. 隐私与可移植

- **静态加密**:SQLCipher(AES-256)护持久关系库(claim/verdict/审计/溯源/笔记),密钥取自 OS keychain。
- **已知不对称(需作为显式安全决策)**:**LanceDB 无透明静态加密**。三选一(按威胁模型):(a) 把 LanceDB/CAS 目录放进 OS 加密卷(FileVault/BitLocker)——契合档 B/C;(b) 应用层文件加密;(c) 视向量索引为"可从加密源重建的明文",接受其落盘明文(当 SQLCipher 源 + OS 盘加密已足够时)。
- **本地优先 by construction**:每项目嵌入式库 + 本地 CAS → 档 B/C 下 claim/证据/verdict **绝不离机/离院**;档 A 只碰模型 API、不碰持久库。
- **可移植/零锁定**:持久库是**开放工件的投影**。主导出 = **RO-Crate(FAIR)** + PROV-O,zip + BagIt/OCFL 让 BLAKE3/SHA 校验随包走;另给 SQLite/CSV/JSONL 直导。**库是便利和索引,绝不是牢笼;文件永远是你拥有的事实。** 导出在 CI 里对 RO-Crate spec + PROV 校验器验证,保证"可移植"真能互操作。

---

## 7. 接入现有架构(零新概念)

- **阅读流水线**的内容寻址清单 → 直接成 durable.db 的 `source` 表;源字节进 CAS;解析/分块/嵌入进 cache.db/LanceDB;按 BLAKE3 差异只重嵌 ~10–15%。
- **claim-证据库** = durable.db 的 claim/evidence/verdict 关系核心 + 混合检索(FTS5 词法 + LanceDB 语义,属 cache)。校验脊柱组件 #1–#4 读写这里。
- **审计链(组件 #13)** = append-only 哈希链事件日志(表 + JSONL 镜像),顺带做再校准数据集 + 可解释性界面 + 崩溃续跑。
- **provenance 链** = PROV-O 关系模型 + 事件日志 + CAS;"无未校验 claim 可渲染"由 emitter **只读 durable.db 里按 claim_id 的 verdict** 强制。
- **记忆层** = 情节日志 + 每项目原子笔记/asks 在每项目库;跨项目持久偏好在全局库——落实"可检索记忆 + 离线再校准、非在线学习""档案柜非扩音器"。
- **部署三档** = 存储 trait 镜像 Rig 角色化 provider 抽象;A/B 用嵌入式 SQLite+LanceDB,C 改配置切 Postgres+pgvector,逻辑同构,agent/校验逻辑零改动。

---

## 8. 需要你拍板的一个安全问题

**向量索引本身需不需要加密?** 还是 "SQLCipher 护源 + OS 盘加密" 就够?
- 我的默认建议:**档 A/B 视向量索引为"可从加密源重建的明文"+ 依赖 OS 盘加密;档 C 用 OS 加密卷兜底**。除非你的威胁模型要求嵌入索引本身也加密(那就在 B/C 强制 OS 卷加密)。
- 这是唯一一处取决于你"要多偏执"的安全姿态,其余我已定。

> ✅ 已定(2026-06-29):采纳默认建议——向量索引视为"可从加密源重建的明文" + 依赖 OS 盘加密(FileVault/BitLocker),机构档用加密卷兜底;**不单独加密向量索引**。

---

## 附录:核验(诚实呈现,含更正)

| # | 判定 | 要点 / 更正 |
|---|---|---|
| 1 | ✅ | Rig v0.39.0 有原生 `rig-sqlite`/`rig-lancedb`/`rig-postgres`(及 qdrant/mongodb/neo4j/surrealdb 等),**无** libSQL/Turso/DuckDB 原生集成。 |
| 2 | ✅ | sqlite-vec 仍 pre-v1 alpha、纯暴力、过 ~10 万向量退化 → 不作研究规模向量库。 |
| 3 | ✅ | LanceDB 嵌入式 ~1M 向量 <10ms、>0.95 召回、低内存 → 推荐的嵌入式向量库。 |
| 4 | ⚠️ | SQLCipher AES-256 经 rusqlite `bundled-sqlcipher` 可用(~5–15% 开销)方向成立;**确切 feature flag/开销编码前再核**。 |
| 5 | ✅ | LanceDB 开源嵌入式**无透明静态加密** → 须在 OS 卷/应用层加密 或 视为可重建明文(显式决策)。 |
| 6 | ⚠️ | "KuzuDB 被 Apple 收购并归档(2025-10)"未证实 → 但"用关系+递归 CTE、不引入嵌入式图引擎"的结论独立成立、不依赖此条。 |
| 7 | ⚠️ | SQLite 递归 CTE 遍历在 <10 万实体/浅深度单位数 ms,深(~depth-6)遍历退化——具体数字未完全证实,本规模够用。 |
| 8 | ✅ | 哈希链是 tamper-evident 非 tamper-proof,**须外部锚定链头**;需免信任第三方核验才上 RFC-6962 Merkle。 |
