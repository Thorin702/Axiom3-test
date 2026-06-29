# Axiom3 — 可施工构建规格(SSOT / Build Spec)

> 版本 v1 · 2026-06-29 · **单一事实源**:本文是开发的权威依据;9 份设计文档降为理由档案,`设计核验报告.md` 为更正清单。
> 已 bake 进核验报告 §1/§2 全部更正 + 两个新决策(分层校验;13 项 MUST 保留并按护城河-demo-先行排序)。
> 锁定栈:Tauri(Rust)+ Rig(rig-core)进程内 + Tokio 监督器 + SQLite(SQLCipher)/LanceDB + MCP 三区网关;仅 macOS+Windows;zh-Hans+en;BYO-API;gated beta;校验为中心。

---

## 〇、规范约定(Normative Conventions —— 冲突时以本节 + §一 schema 为准)

> 4 节并行起草产生过跨节命名/类型分叉,以下为权威裁定。**§一 数据 schema 是唯一真值;§二/§三 的 Rust 类型是其进程内投影,命名/类型冲突一律以 §一 + 本节为准。**

1. **ID 类型 = ULID**(单调可排序,利于 event 排序与主键索引)。全文出现的 "UUID/Uuid" 一律按 ULID 解。
2. **`verdict.status` 严格 5 态** = `{verified, refuted, uncertain, quarantined, injection_neutralized}`。**`evidence_redacted` 是 verdict 上的派生布尔列,不是第 6 个枚举值**;全文"转 EvidenceRedacted 态"一律读作"置 `evidence_redacted=1`,`status` 不变"。
3. **`claim_type` 权威并集** = `{citation, numeric_stat, causal, methodological, definitional, descriptive, synthesized, non_retrievable}`(`synthesized` 供 §二阶段12 新合成 claim 重入校验落库;`non_retrievable` 供 §三 #9 接地路径)。
4. **`independence` 三态** = `{multi_family, single_family, local_only}`。承重 claim 若 <2 独立模型家族,E2 不得 `verified` → 强制 `uncertain` + `uncertain_reason='single_family_independence_unmet'`。
5. **校验流水线 = 12 阶段**(设计对齐旧核 10 步 + 2 个检索侧前置阶段)。全文"10 阶段"一律指此 12 阶段。
6. **`capability_matrix` 主键 = `(provider, model_id)`**。连接锚点 = per-provider(凭据/端点);能力矩阵行 = 经 `/models` 发现的 per-model 结果。"连 provider 非版本"与"按 model 缓存能力"不矛盾。
7. **`source` 表 day-one 含 `license`**(DSM Art.4/Art.3 法务硬门用)+ `acquisition_channel`;P0 schema 清单据此(兑现"法务从第一天进 schema")。
8. **`PENDING` 是 claim 的处理生命周期态(运行时),≠ `verdict.status`(持久 5 态)**;barrier/loop 条件引用 claim 处理态,不引用 verdict.status。
9. **离线硬完整性门(Crossref/Retraction Watch)= fail-closed**(标 `uncertain` 不放行)——本 SSOT 拍板,非待决。
10. **抗投毒措辞中性化**:表述为"生成/解析职责分离 + 隔离解析器只回带引用的类型化证据";**全文不引用 CaMeL 67%**,对外只说 poison-resistant / 可审计,**绝不 injection-proof**;保证来自确定性入口门 + 污点/provenance 标签 + 红队 harness 实测(非能力追踪解释器)。
11. **`UsageRecord` 以 §一 1.2.4 为准**(必含 `task_id`、`cache_read_input_tokens` —— 后者是 P0 头号风险 spike "Rig cache_control 命中验证"的判据)。
12. **EU AI Act 第 50 条标注归"披露/导出"组件**(§二阶段12 披露引擎),不挂在抗投毒护盾面板;护盾面板只放投毒/撤稿/隔离统计。
13. **图表/可视化能力(2026-06-29 定,按 provenance 分裂,非"一个画图功能")**:**支持** ①数据图/科研图(代码→图:matplotlib/plotly/vega-lite,从已校验数据)②结构图/流程图(图即代码:Mermaid/Graphviz/D2/TikZ;含 claim-证据图 / provenance / workflow 可视化——本已是 MUST)③科学结构渲染(RDKit/PyMOL 等,从数据/模型)。**铁律**:每张产出图必须**可复现(随附 code+data+seed)+ 带 provenance**,与 claim 同样过 provenance 门控 emit;从图表抽取的数字带复核 flag。**不做**:无接地的 AI 生成插画/"效果图"(text→image,无出处、期刊禁 AI 图、与验证品牌相反);若用户坚持,须显式标"AI 生成、无证据支撑"且默认不入可发表文档。图在输入侧亦为一等公民(图表理解摄入 + 图像篡改取证),仅"无接地生成"被排除。**(待用户确认"效果图"指数据/模型渲染[支持] 还是 AI 凭空插画[排除]。)**

---

## 一、总体架构 + 数据模型/Schema(已修正)

> SSOT §1。本节是可照建的规格:分层架构 + 模块边界 + 字段级 schema。所有内容已并入设计核验报告 §1/§2 全部更正(5 态 verdict、能力矩阵带 status/sunset、三 i18n 轴、删除/编辑态、BYO、分层校验、零推理后端 + 极小身份服务、单家族独立性闸门、模态仅感知向)。锁定栈:Tauri(Rust)+ Rig(rig-core)进程内 + Tokio 监督器 + provider 抽象 + SQLite(SQLCipher)/LanceDB + MCP 三区网关 + 校验引擎 + 身份/更新后端;仅 macOS+Windows;zh-Hans+en。

## 1.1 总体架构(分层,文字图)

```
┌──────────────────────────────────────────────────────────────────────────┐
│ A. Webview 前端 (React + i18next + i18next-icu)                            │
│    任务看板(主屏,反聊天) · claim 中心视图 · provenance/审计可视化         │
│    抗投毒护盾面板 + 完整性成绩单 · 阅读/摄入区 + 隔离队列 · Asking 收件箱   │
│    BYO 密钥/部署向导 · 用量/成本面 · 三 i18n 轴控件 · 文档生成+导出         │
├──────────────────────────────────────────────────────────────────────────┤
│ B. Tauri IPC: commands(请求) + ipc::Channel<T>(每 agent 有序流式)         │
├──────────────────────────────────────────────────────────────────────────┤
│ C. 场景注册表(8+1 场景插件,统一 4 段契约 a/b/c/d)                         │
├──────────────────────────┬───────────────────────────────────────────────┤
│ C1 专家模式               │ C2 Dynamic Workflow 模式                        │
│ 单 agent(Rig)深度循环    │ Tokio 监督器派发 N×Rig agent(见 D)            │
├──────────────────────────┴───────────────────────────────────────────────┤
│ D. 编排监督器 (Tokio): Semaphore/JoinSet 有界并发 · 令牌桶准入(per         │
│    provider/model/限流维度) · 两层硬成本上限 · 幂等键+DLQ · 事件日志续跑   │
│    + 声明式 DAG/状态机控制平面(fan-out/barrier/loop)+ Asking interrupt/   │
│    resume checkpointer(机制已全,DAG schema 见后续 SSOT 节)               │
├──────────────────────────────────────────────────────────────────────────┤
│ E. 校验引擎(脊柱) — 分层校验:                                            │
│    E1 确定性/符号检查器(对【所有】claim:引用解析/撤稿门/statcheck/GRIM/   │
│       重算/单位/设计感知因果约束/spin/视觉一致性入口门) — 便宜、可守      │
│    E2 LLM 对抗舰队(【仅】承重/结论 claim:红队反驳/跨家族二审/裁决)        │
│    E3 校准模块(由校验结果导出置信,报 ECE,conformal 弃权阈值)            │
│    跑前给 “~$X / ~N agents / 在你限额下 ~T 小时” 预估(见 1.1.1)            │
├──────────────────────────────────────────────────────────────────────────┤
│ F. Agent 引擎: Rig(turn/tool/streaming)+ 沙箱/审批(per-OS,见 1.1.2)    │
├──────────────────────────────────────────────────────────────────────────┤
│ G. MCP 网关(三区信任 Core/Curated/Community,唯一工具咽喉)                 │
│    描述+输出皆不可信 · 哈希钉死+漂移告警 · 沙箱 · 双向 DLP · provenance     │
├──────────────────────────────────────────────────────────────────────────┤
│ H. 模型接入层(provider 抽象): 内部规范模型(Anthropic 富块结构)→         │
│    OpenAI / Anthropic / OpenAI 兼容 / 本地 适配器 · 连【provider】非版本    │
│    · /models 发现 → 能力矩阵 · 角色路由(quality-first)· 独立性闸门        │
│    · 模态路由器(仅感知/摄入向:视频理解/音频转写 + 跨模态校验)            │
├──────────────────────────────────────────────────────────────────────────┤
│ I. 存储层(trait: DurableStore + VectorStore + EventLog)                   │
│    .workbench/ 内: durable.db(SQLCipher,备份)+ cache.db(可重建)         │
│    + LanceDB(可重建)+ cas/(BLAKE3 字节)+ audit/(JSONL 镜像)            │
│    服务器选项: Postgres + pgvector(同一逻辑 schema,改配置非重写)         │
├──────────────────────────────────────────────────────────────────────────┤
│ J. 横切: keyring(OS keychain)· 用量/成本计量(UsageRecord)· 事件日志/   │
│    续跑 · 污点/provenance 标签 · 诚信治理 · 可观测                          │
├──────────────────────────────────────────────────────────────────────────┤
│ K. 可选 sidecar(默认关,非 fan-out 主路径): Codex app-server(科研编码,  │
│    场景5,延后)/ Claude Agent SDK CLI(Anthropic 保真)                    │
└──────────────────────────────────────────────────────────────────────────┘

  独立后端(进程外,极小,永不碰研究数据):
   L1 身份/审批服务   — 手机 OTP 注册 + 创始人审批队列【仅此二者】
   L2 动态更新服务器  — 通道/灰度/强制/回滚(204 或小签名 JSON,可 CDN 前置)
   L3 静态签名工件    — GitHub Releases 整包 + minisign;MCP/学科包 + 投毒
                        检测规则 feed(版本化签名 JSON,客户端钉死公钥)
```

### 1.1.1 分层校验(修正核验报告 §1#3:取消“每 claim 全舰队”)

| 层 | 适用范围 | 内容 | 成本/独立性 |
|---|---|---|---|
| E1 确定性/符号 | **所有** claim(便宜、可守、有地面真值处质量更高) | 引用解析(Crossref→DataCite/OpenAlex/S2)、撤稿硬门、statcheck/GRIM/GRIMMER/SPRITE 软标记、数字/单位重算、设计感知因果越界、四轴 spin、摄入视觉一致性门 | 本地/确定性,不计 LLM token |
| E2 LLM 对抗舰队 | **仅** `is_bearing=true`(承重/结论)claim | 红队反驳(证伪闸门)、跨独立家族二审、裁决器(按证据非投票) | 计 token;**需 ≥2 独立模型家族** |
| E3 校准 | 全部已判 claim | 由校验结果导出置信,报 ECE,conformal 弃权阈值;压制模型自报置信 | — |

- **跑前预估(必显):** 监督器在派发前用令牌桶余量 + 单 agent token 估算产出 `RunEstimate{est_usd, est_agents, est_minutes_under_quota}`,展示在任务看板提交确认页。
- **独立性闸门(修正 §2):** 若已连 **<2 个独立模型家族**,承重 claim 的 E2 不得打 `verified`,强制 `verdict.status=uncertain` 且 `verdict.uncertain_reason='single_family_independence_unmet'`,Trust-UX 以 `independence` 一等字段显示。

### 1.1.2 per-OS 沙箱(修正核验报告 §3,非 Linux 容器)

| OS | 机制 | 可见强度指标 |
|---|---|---|
| macOS | App Sandbox / seatbelt profile + entitlements(硬化运行时) | `sandbox_strength` 枚举 |
| Windows | AppContainer + job objects + WFP(网络过滤) | 同上(per-OS 不同上限) |

沙箱强度作 per-OS 可见指示;首发审批/沙箱只需覆盖**代码执行确定性检查器**(单一受控动作),computer-use 全审批栈 DEFER。

## 1.2 数据模型/Schema(可今日实现)

### 1.2.1 durable.db ↔ cache.db 划分(唯一规则:可重建性)

| 库 | 内容 | 加密 | 备份 |
|---|---|---|---|
| `durable.db`(SQLite STRICT + SQLCipher AES-256) | claim/source/evidence_span/verifier/verdict/连接表/refuter_attempt/adjudication/human_decision/quarantine_event/usage_record/note_ask/calibration_history/source_manifest/event | 是(密钥 OS keychain) | **必须** |
| `cache.db`(SQLite STRICT) | 解析/TEI/表 JSON/转写、分块、FTS5 词法索引、图投影 | 否 | 从不(一键重建) |
| `LanceDB` 目录 | 嵌入 + ANN(IVF_PQ) | 否(视为可从加密源重建的明文 + OS 盘加密) | 从不 |
| `cas/`(文件) | 源字节,按 BLAKE3 哈希寻址 | OS 盘加密 | 随项目目录 |
| `audit/`(JSONL) | 事件日志 append-only 镜像(git-diff/grep 友好) | OS 盘加密 | 随项目目录 |

### 1.2.2 核心实体(durable.db,STRICT 表)

**`source`** — 内容寻址的源清单(= 源 manifest 表)
| 字段 | 类型 | 说明 |
|---|---|---|
| `source_id` | TEXT PK | UUID |
| `blake3_hash` | TEXT NOT NULL | CAS 寻址键;**crypto-shred 后仍保留,使哈希链有效** |
| `uri` | TEXT | 原始 URI(可 link-rot) |
| `mime` | TEXT | |
| `acquisition_channel` | TEXT | 获取渠道标签(开放获取/订阅/Crossref 礼貌池/Unpaywall/BYO key…),供法务+校验 |
| `source_locale` | TEXT | **i18n 轴3**:自动检测,喂隔离 |
| `data_class` | TEXT | per-project 数据类声明(undeclared 保守默认 / PII / PHI…) |
| `retracted_status` | TEXT | clean/retracted/concern;变更触发 provenance 再评估 |
| `cas_state` | TEXT | `present` / `crypto_shredded`(RTBF) |
| `created_at` | INTEGER | epoch ms |

**`claim`** — 原子断言(核心对象)
| 字段 | 类型 | 说明 |
|---|---|---|
| `claim_id` | TEXT PK | |
| `project_id` | TEXT NOT NULL | per-project 作用域 |
| `text` | TEXT NOT NULL | 原子断言文本 |
| `claim_type` | TEXT | citation / numeric_stat / causal / methodological / general |
| `is_bearing` | INTEGER NOT NULL | 是否承重/结论(决定是否触发 E2 舰队) |
| `output_locale` | TEXT | **i18n 轴2**:per-project,注入 agent 提示 |
| `created_at` | INTEGER | |

**`evidence_span`** — 钉在不可变源字节+偏移
| 字段 | 类型 | 说明 |
|---|---|---|
| `evidence_id` | TEXT PK | |
| `source_id` | TEXT FK→source | |
| `blake3_hash` | TEXT NOT NULL | 冗余钉死(源变了 verdict 仍可验) |
| `start_offset` / `end_offset` | INTEGER | 字节偏移 |
| `verbatim_text` | TEXT | 逐字文本;**RTBF 时可 null(`redaction_state`),保留 hash** |
| `parse_trust` | REAL | 解析置信(低 → 隔离) |
| `redaction_state` | TEXT | `present` / `redacted`(许可下架或用户删) |

**`verifier`** — 校验者身份(人 或 模型)
| 字段 | 类型 | 说明 |
|---|---|---|
| `verifier_id` | TEXT PK | |
| `kind` | TEXT NOT NULL | `human` / `model` |
| `model_id` | TEXT | 模型时:对应能力矩阵 |
| `model_family` | TEXT | 独立性判定用 |
| `version` | TEXT | |
| `prompt_hash` | TEXT | 模型时:prompt 内容哈希 |
| `role` | TEXT | reading/reasoning/routine_verify/adversarial/adjudicate/modality |

**`verdict`** — 不可变、版本化;**5 态枚举(修正核验报告 §1#4)**
| 字段 | 类型 | 说明 |
|---|---|---|
| `verdict_id` | TEXT PK | |
| `claim_id` | TEXT FK→claim | |
| `verifier_id` | TEXT FK→verifier | |
| `status` | TEXT NOT NULL | **CHECK ∈ {`verified`,`refuted`,`uncertain`,`quarantined`,`injection_neutralized`}** |
| `evidence_redacted` | INTEGER | 派生状态:支撑证据被编辑时置 1(供 Trust-UX 降级显示) |
| `calibrated_confidence` | REAL | 由 E3 导出,非模型自报 |
| `uncertain_reason` | TEXT | 如 `single_family_independence_unmet` / `evidence_insufficient` |
| `independence` | TEXT | `multi_family` / `single_family` / `local_only` |
| `revises_verdict_id` | TEXT FK→verdict | PROV `wasRevisionOf`;重判=插新行,绝不静默升级 |
| `created_at` | INTEGER | |

> verdict 词汇 = 锁定 i18n 命名空间:`status` 即**语言无关机器码**;另由前端 glossary 命名空间提供 (颜色/图标 token, 本地化 label, 本地化 tooltip);zh 标签待领域专家签核。

### 1.2.3 连接表 / 边(claim 图,递归 CTE 遍历)

| 表 | 关键字段 | 说明 |
|---|---|---|
| `verdict_uses_evidence` | `verdict_id` FK, `evidence_id` FK, `stance ∈ {supports,refutes}` | verdict↔证据;每 verdict 必引具体 span |
| `claim_relation` | `parent_claim_id`, `child_claim_id`, `type ∈ {subclaim,supports,contradicts}` | claim 图边 |
| `refuter_attempt` | `attempt_id` PK, `claim_id`, `verifier_id`, `outcome ∈ {refuted,failed_to_refute}`, `rationale`, `evidence_id?` | 红队反驳尝试(证伪闸门) |
| `adjudication` | `adjudication_id` PK, `claim_id`, `method='evidence_tree_audit'`, `winning_verdict_id`, `rationale` | 分歧点按证据裁决 |
| `human_decision` | `decision_id` PK, `claim_id?`, `checkpoint_type`, `actor`, `decision`, `is_override`, `friction_level`, `note`, `ts` | 人工检查点/覆盖(高摩擦留痕) |

### 1.2.4 新增实体(修正核验报告 §1#4 + §2 + RTBF)

**`quarantine_event`** — 隔离非删
| 字段 | 类型 | 说明 |
|---|---|---|
| `quarantine_id` | TEXT PK | |
| `source_id` | TEXT FK→source | 被隔离源 |
| `trigger_rule` | TEXT | 触发的入口/检测规则 id(连规则 feed 版本) |
| `rule_feed_version` | TEXT | 检测规则包版本 |
| `category` | TEXT | hidden_prompt / retracted / tortured_phrase / metadata_anomaly / low_parse_trust… |
| `disposition` | TEXT | `quarantined` / `reviewed_admitted` / `injection_neutralized` |
| `created_at` | INTEGER | |

**`usage_record`** — 归一化计费/用量(修正核验报告 §1#4,跨模态)
| 字段 | 类型 | 说明 |
|---|---|---|
| `usage_id` | TEXT PK | |
| `project_id` | TEXT | |
| `task_id` | TEXT | |
| `modality` | TEXT | text / audio_transcription / video_understanding(生成路径保留字段、首发不暴露) |
| `provider` | TEXT | |
| `model_id` | TEXT | |
| `input_units` / `output_units` | INTEGER | $/MTok、$/audio-token、$/min、$/sec 的原始计数 |
| `cache_read_input_tokens` | INTEGER | 验证缓存命中 |
| `normalized_usd` | REAL | 统一货币口径,供按成本路由 + 预算 |
| `ts` | INTEGER | |

**`calibration_history`** — 再校准数据集
| 字段 | 类型 | 说明 |
|---|---|---|
| `calib_id` PK / `model_id` / `model_version` / `role` / `ece` / `conformal_threshold` / `held_out_set_hash` / `computed_at` | | 漂移触发重验 |

**`note_ask`** — 记忆层持久侧(笔记 / Asking)
| 字段 | 类型 | 说明 |
|---|---|---|
| `note_id` PK | TEXT | |
| `project_id` | TEXT | per-project(证据/verdict 绝不跨项目) |
| `kind` | TEXT | `ask`(系统→人) / `annotation`(人→项目记忆) |
| `claim_id` | TEXT? | 关联 claim |
| `taint` | TEXT | 污点标签(粘自投毒源的批注同样标记) |
| `interrupt_token` | TEXT? | 接监督器 interrupt/resume checkpointer |
| `created_at` | INTEGER | |

### 1.2.5 防篡改:append-only 哈希链事件日志 + PROV-O

**`event`**(durable.db,且镜像 `audit/*.jsonl`)
| 字段 | 类型 | 说明 |
|---|---|---|
| `seq` | INTEGER PK AUTOINCR | 单调序号 |
| `ts` | INTEGER | |
| `actor` | TEXT | human / model / system |
| `op` | TEXT | 操作类型(含 `forget_propagation`、`crypto_shred`、`evidence_redact`、`quarantine`、`verdict_revise`) |
| `payload_json` | TEXT | 规范化 JSON |
| `prev_hash` | TEXT | 前一条 entry_hash |
| `entry_hash` | TEXT | `H(prev_hash ‖ canonical(payload))`,BLAKE3 |

- 当前状态 = 从事件日志重放的投影(事件溯源,顺带崩溃续跑)。
- **链头哈希定期外部锚定**(git/打印);哈希链是 tamper-evident 非 tamper-proof。第三方免信任核验才上 RFC-6962 Merkle。
- **PROV-O 映射(导出投影,非新存储):** `source`/`evidence_span`/派生工件 → `prov:Entity`;校验/解析/裁决步骤 → `prov:Activity`;`verifier` → `prov:Agent`;边映射 `wasGeneratedBy`/`used`/`wasDerivedFrom`/`wasRevisionOf`(verdict.revises)。RDF 经可选 Oxigraph 导出,主导出 = RO-Crate(FAIR)+ PROV-O。

### 1.2.6 删除 / RTBF(修正核验报告 §3)

| 操作 | 机制 | 链/状态影响 |
|---|---|---|
| 用户删除 / 许可下架(区分二者) | `cas`:crypto-shred 字节,**保留 BLAKE3**;`evidence_span.verbatim_text`:null + `redaction_state=redacted` | 哈希链仍有效 |
| 派生 verdict | 置 `evidence_redacted=1`(新增 verdict 态语义),Trust-UX 降级显示 | 不可变 verdict 不删,加注 |
| 传播 | `event(op='forget_propagation')` 审计事件,级联到所有引用该源的 claim | 留痕 |
| 保留 | per-data-class 保留策略;`source.data_class` 驱动 | — |

### 1.2.7 作用域(per-project + global)

| 作用域 | 库 | 内容 |
|---|---|---|
| per-project(默认) | `.workbench/`(durable.db + cache.db + LanceDB + cas + audit) | 一切证据/claim/verdict/隔离/笔记;**密钥按项目**;自包含可移植 |
| global / user | `~/.config` 级小 SQLite | 跨项目**非证据**:UI 偏好(uiLocale)、可泛化校准先验、能力矩阵缓存、API 凭据**指针**(真 key 在 keychain)、已知项目索引 |

口诀:是证据/verdict → per-project(绝不跨泄);是偏好/可复用校准 → global。

### 1.2.8 三条 i18n 轴字段(day-one 进 schema)

| 轴 | 字段位置 | 默认/行为 |
|---|---|---|
| `uiLocale`(全局) | global SQLite(user 偏好);初值 `tauri-plugin-os::locale()` | en / zh-Hans 单轴切换 |
| `outputLocale`(per-project) | `claim.output_locale`(项目级默认表) | 跟随 UI 可改;**注入 agent 系统/任务提示**;数字/日期/CSL 引文跟此轴 |
| `sourceLocale`(per-document) | `source.source_locale` | **自动检测**;低置信非拉丁/CJK → quarantine_event;首发检测-only,手动覆盖槽预留 |

### 1.2.9 能力矩阵 schema(连 provider 非版本)

**`capability_matrix`**(global SQLite 缓存,经各家 `/models` 发现 + 漂移监视器刷新)
| 字段 | 类型 | 说明 |
|---|---|---|
| `provider` | TEXT | 连接的 provider(openai / anthropic / openai_compatible / local) |
| `model_id` | TEXT PK(复合 provider) | |
| `model_family` | TEXT | **独立性判定**(<2 家族 → 承重 claim uncertain) |
| `input_modalities` | TEXT(JSON) | text/image/audio/video(感知向) |
| `output_modalities` | TEXT(JSON) | 首发仅 text;生成模态保留字段不暴露 |
| `features` | TEXT(JSON) | reasoning / function_calling / mcp / prompt_cache / signed_thinking / native_av_sync(保留) |
| `transports` | TEXT(JSON) | rest / async_job(websocket/webrtc/sip 保留,实时双工 DEFER) |
| `limits` | TEXT(JSON) | ITPM / OTPM / RPM |
| `cost` | TEXT(JSON) | $/MTok、缓存读写非对称、$/sec、$/audio-token(后两者保留字段) |
| `status` | TEXT | **`active` / `suspended` / `sunset`**(捕捉 Fable 5 这类活例) |
| `sunset_date` | INTEGER? | 退役日;路由器自动剔除(如 Sora);**Claude Fable 5 标 `suspended`,不列为可路由,前沿推理/裁决回退 Opus 4.8** |
| `refreshed_at` | INTEGER | |

> 修正核验报告 §1#1:不列 Fable 5 为当前可路由;能力矩阵的 status/sunset 正是为此设计。Claude 当前文本+图输入/文本输出族:Opus 4.8(前沿推理/裁决回退)、Sonnet 4.6、Haiku 4.5。

### 1.2.10 provider / role 路由表(quality-first,自动 + 可覆盖)

**`role_routing`**(global SQLite)
| 字段 | 类型 | 说明 |
|---|---|---|
| `role` | TEXT PK | reading / reasoning / routine_verify / adversarial / adjudicate / modality |
| `selection_mode` | TEXT | `auto`(默认,按 features+quality 解析)/ `pinned`(高级覆盖) |
| `pinned_model_id` | TEXT? | 覆盖时手动钉 model→role |
| `quality_first` | INTEGER | 默认 1(质量优先于成本/延迟) |
| `fallback_order` | TEXT(JSON) | 4xx 不匹配 / 429 / 5xx 时回退候选 |

- 连**provider**(非模型版本);app 经 `/models` 发现填充能力矩阵;按 `role` 所需 features 自动解析候选,quality-first 排序,可高级覆盖 pin。
- 模态路由器仅感知向:`reading`/`modality` 角色解析 `感知适配器 → 文本推理器`(视频理解/音频转写)+ 跨模态校验;视频生成/实时双工不解析(能力矩阵保留字段,$/sec、$/audio-token 备用)。
- 敏感数据 provider 路由硬闸 + 钉死每 agent 可调用 provider+端点(投毒文档无法重定向,接威胁模型 §5)。

---

## 二、文献调研垂直 · 端到端流程(首发 P1)

> 本节是文献调研垂直的**可照建**端到端规格(开发者据此实现 P1)。它把校验脊柱(设计对齐 §2 的 13 组件)与阅读流水线(手与眼 §1)、存储层(私有DB §4)、威胁模型(威胁模型 §1 八攻击面)、分层校验(设计核验报告 §1 必修#3 已采纳)缝成一条带类型化 I/O 的有序管线。本节为**首发 P1 唯一垂直**(设计对齐 §6.2 锁定:首批垂直=通用文献调研)。
>
> **贯穿不变式(全程成立,不在单阶段重复):**
> - **生成不可采信(inadmissible)**:任何 LLM 生成的 claim 在通过独立、对抗、基于证据的校验前不得渲染入文档(设计对齐 §0)。
> - **provenance 是强制横切**:每阶段发出 PROV-O 记录(`数据→变换→结果→claim→源`),写入 append-only 哈希链事件日志(私有DB §4)。法务(获取渠道)、安全(注入/隔离状态)、撤稿状态共用**同一条** provenance 脊柱(威胁模型 §6 盲点④),不建三套。
> - **限流是安全控件**:Tokio 令牌桶限流在每阶段对 LLM 与文献 API 调用生效,既防 BYO key 被 429,又是注入引发 agent 爆炸的爆炸半径限制器(威胁模型 §0)。
> - **诚信层凌驾双向**:用户指令与投毒源同样不能授权造假/p-hack/伪造引用(评估-Asking §3)。

---

## A. 数据类型词汇表(全阶段共用,先定义后引用)

```rust
// 源与证据(durable.db,STRICT 表;私有DB §4)
struct Source {
  id: Uuid,
  blake3_hash: [u8;32],        // CAS 字节地址,源 link-rot 后证据仍可验
  uri: String, mime: String,
  acquisition_channel: AcqChannel,   // 法务标签,见阶段5
  retraction_status: RetractionStatus,
  license: LicenseTag,               // DSM Art.4 / Art.3 / 未声明保守默认
  source_locale: Option<LangCode>,   // 自动检测,喂隔离(i18n 第三轴)
}
enum AcqChannel { PublisherOA, Unpaywall, RepositoryGreenOA, PublisherLicensed, UserUpload, Pirate /*硬拦*/ }
enum RetractionStatus { Clean, ConcernExpressed, Retracted }

struct EvidenceSpan {        // 钉在不可变源字节+偏移上
  source_id: Uuid, blake3_hash: [u8;32],
  offset: SpanLocator,       // 页/坐标框/单元格/时间戳/行
  verbatim_text: String,     // RTBF 时可 null 化但保 hash(决策#12)
  parse_trust: TrustLevel,   // 解析器置信包络,非自报
  modality: Modality,        // text|table|figure|audio|video|code
}

// claim 与判定(durable.db,verdict 不可变版本化)
struct Claim {
  id: Uuid, text: String,
  class: ClaimClass,         // 见阶段8
  bearing: bool,             // 承重/结论 claim 才上 LLM 舰队(分层校验关键位)
  project_id: Uuid, output_locale: LangCode,
}
enum ClaimClass { Citation, Statistical, Causal, Methodological, Descriptive, Synthesized }

enum Verdict {               // 5 态枚举(决策#4,语言无关机器码 + 颜色/图标 token)
  Verified, Refuted, Uncertain, Quarantined, InjectionNeutralized,
  // EvidenceRedacted 为 RTBF 后的派生态(决策#12),由 emit 层处理
}
struct VerdictRecord {
  claim_id: Uuid, verifier_id: Uuid, status: Verdict,
  calibrated_confidence: f32,        // conformal,非模型自报
  independence: IndependenceTag,     // 见阶段9
  revises_verdict_id: Option<Uuid>,  // PROV wasRevisionOf,重判=插新行
}
enum IndependenceTag { CrossVendor, SingleFamilyUnmet, LocalSingle }

// 隔离与计费(决策#4 新增实体)
struct QuarantineEvent { source_id: Uuid, trigger_rule: RuleId, reason: String, /* 隔离非删 */ }
struct UsageRecord { modality: Modality, provider: String, model: String,
                     input_units: u64, output_units: u64, normalized_usd: f64 }
```

---

## B. 端到端 12 阶段(每阶段:输入类型 → 输出类型 / 组件与工具 / 失败处理)

### 阶段 1 · 任务摄入 + outputLocale 绑定
- **In**: `ResearchTask { query_nl: String, project_id, scope: DataClassDeclaration }`
- **Out**: `NormalizedTask { intent, output_locale: LangCode, data_class }`
- **组件**: 项目工作区(桌面端 MUST);outputLocale 取项目设置默认跟随 uiLocale,**注入 agent 系统提示**(非仅格式化,桌面端 §5)。data_class 未声明→保守默认 + PII/PHI 自动检测(决策#14)。
- **失败**: 未声明数据类→不阻塞但落保守档;检出 PII/PHI→标记并影响后续 provider 路由硬闸。

### 阶段 2 · 查询扩展
- **In**: `NormalizedTask`
- **Out**: `ExpandedQuery { terms[], synonyms[], filters{year,venue,lang}, mesh_terms[] }`
- **组件**: 一个轻量 LLM(reading/routine 角色,非前沿)生成同义词/MeSH/检索算子。**此阶段产物是检索输入,不是 claim**——不进校验。
- **失败**: 扩展为空→回退原始 query;过宽(预估命中>上限)→提示用户收窄或加过滤。

### 阶段 3 · 多源检索
- **In**: `ExpandedQuery`
- **Out**: `Vec<SearchHit { external_id, doi?, title, authors, year, abstract?, source_api }>`
- **组件/工具**: OpenAlex / Crossref / PubMed (NCBI E-utilities) / Semantic Scholar,经 MCP 网关 Core 区(paper-search-mcp 式)。
- **凭据(决策#5)**: **BYO key 存 OS keychain**;零摩擦默认 = Crossref 礼貌池(仅 UA)+ Unpaywall(仅邮箱)。OpenAlex/NCBI/S2 的 key 为"可选但更强"。
- **限流(法律事实,决策#14)**: OpenAlex = **信用制**($1/天免费、按端点信用、耗尽返 **409**)——非 100 req/s;NCBI ~10 rps;S2 ~1 rps。令牌桶按**每用户每 key**镜像各家桶。
- **失败**: 单 API 409/限速→降级到仍有额度的源并标记覆盖缺口(诚实展示,非静默);无 key→自动只用 Crossref+Unpaywall 默认池。

### 阶段 4 · 去重 + 排序
- **In**: `Vec<SearchHit>`
- **Out**: `Vec<RankedHit { hit, rank_score, dedup_cluster_id }>`
- **组件**: DOI 优先去重 + 标题/作者/年份模糊匹配(CiteCheck 三分类基线:精确/轻损/编造);排序按相关性 + 召回/精度优化档(场景 4 段契约 (c))。
- **失败**: 冲突元数据(同 DOI 不同标题)→上报而非任选其一。

### 阶段 5 · 全文抓取 + 获取渠道标签(法务硬门)
- **In**: `Vec<RankedHit>`
- **Out**: `Vec<Source>`(已落 CAS 字节 + `acquisition_channel` + `retraction_status` + `license`)
- **组件/工具**: Unpaywall/绿色 OA/出版商 OA 解析 → 字节存 `.workbench/cas/`(BLAKE3 寻址,私有DB §2)。
- **硬规则**:
  - **盗版源硬拦**(`AcqChannel::Pirate` 永不抓取/入库)——法务红线。
  - **撤稿筛**: Crossref REST `update-to` + Retraction Watch 数据 → `Retracted` 硬拒进 claim 库;`ConcernExpressed`(PubPeer)**只标记不拒**(设计对齐 §点2)。
  - **license/片段限制**: 商业+研究默认 **DSM Art.4**(尊重机器可读 opt-out);Art.3 研究例外为**显式 opt-in**(决策#14);无许可信息→保守按可引用片段长度限制存储。
- **失败**: 仅有付费墙摘要→以 abstract-only 入库并降信任级;license 不明→保守片段模式 + flag。

### 阶段 6 · 摄入门(抗投毒第一关 → 隔离队列)
- **In**: `Vec<Source>` + 各源解析产物(阅读流水线 手与眼 §1 步2-3:GROBID/MinerU/表格/图表/音视频专家解析器,**通用 LLM 不做主 OCR**)
- **Out**: `IngestVerdict { admitted: Vec<EvidenceSpan>, quarantined: Vec<QuarantineEvent>, to_human: Vec<...> }`
- **组件(确定性 + 取证,威胁模型 §1 攻击面1)**:
  - **视觉一致性不变量**: 比对"LLM 可见字符流"vs"人类可见渲染",命中白底白字/微字体/页外文本/cmap 重映射/隐藏 HTML 注释 → **隔离,永不进 claim 库**。
  - **隐藏提示扫描** + tortured-phrase + 元数据异常。
  - **统计取证**: statcheck(p 值重算)/GRIM/GRIMMER/SPRITE(注意非同一 R 包,设计对齐 附录A#6)——**软标记转人工,不自动拒**(有记录在案误报)。
  - **DOI 解析**: 不存在 DOI → 硬拒(LLM 文献表存在率仅 40-50%,GhostCite)。
  - **源语言轴**: 低置信非拉丁/CJK 抽取(HTR ~83% 字准<可用线)→ 自动进隔离转人工(桌面端 §5)。
- **隔离语义**: **隔离非删除**(QuarantineEvent 持久);verdict 候选标 `Quarantined`;成功消解的工具注入标 `InjectionNeutralized`。
- **失败处理(关键决策点)**: **离线时硬完整性门(Crossref/Retraction Watch 跑不了)→ fail-closed**:无法验证撤稿状态的源标 `Uncertain` 不得自动采纳为 verified(此为剩余设计议程列出的待决,P1 取保守 fail-closed 默认 + 可见标记)。

### 阶段 7 · 提案(inadmissible)
- **In**: `admitted: Vec<EvidenceSpan>` + `NormalizedTask`
- **Out**: `Vec<ProposedStatement>`(标记 `admissible=false`)
- **组件**: 生成 LLM 仅作"提案源",**grounded in 检索证据**(全文 RAG / PaperQA2 式),输出必须携带候选证据 span 指针。**dual-LLM 隔离**(威胁模型 §1 攻击面2):特权规划 LLM 永不读原始投毒内容,隔离解析 LLM 只回带引用的类型化证据。
- **失败**: 生成句无可锚定证据 span → 不进入下游(不铸造无源连接句)。

### 阶段 8 · claim 拆解 + 分类
- **In**: `Vec<ProposedStatement>`
- **Out**: `Vec<Claim>`(原子、带 `class` + `bearing` 标志)
- **组件**: Claim 拆解器(FActScore 式原子化,设计对齐 组件1);分类器打 `ClaimClass` 并判定 `bearing`(结论句/承重 claim = true)。**bearing 标志是分层校验的路由开关**。
- **失败**: 无法原子化的复合句→拆到可独立核查粒度;分类不确定→保守按 bearing=true(多校验不少校验)。

### 阶段 9 · 分层校验(TIERED — 已采纳,替换"全程每 claim 全舰队")
> **核心修正(决策#1 / 设计核验报告 §1 必修#3):**
> - **确定性/符号检查器对 *所有* claim** 跑(便宜、有地面真值处质量更高、可守)。
> - **LLM 对抗舰队仅对 `bearing=true` claim** 跑。
> - **跑前给预估**:`~$X / ~N agents / 在当前 key 限额下 ~T 小时`(桌面端 鲁棒性面 MUST)。

- **In**: `Vec<Claim>` + 证据
- **Out**: `Vec<VerdictRecord>`(每条 verdict 必引用具体 EvidenceSpan)

| claim 路由 | 校验器(确定性层,全 claim) | LLM 舰队层(仅 bearing) |
|---|---|---|
| Citation | 引用解析(Crossref→DataCite→OpenAlex→S2)+ ALCE 引文精/召 | 对抗红队找编造/误引(FACT 式抓源确认支撑) |
| Statistical | 代码执行重算 p 值/自由度/单位/多重比较校正 | — (确定性已定胜负,除非分歧升级) |
| Causal | 设计感知约束分类器(非 RCT 禁因果动词;**绝不让 LLM 自查因果**,Corr2Cause F1~29) | 红队找混杂 |
| Methodological / Descriptive | 逐 claim NLI 接地(MiniCheck/Bespoke-MiniCheck-7B,GPT-4 级 1/400 成本) | 跨厂商二审 |
| 不可检索 | SelfCheckGPT-NLI(便宜默认 ~92 AUC-PR) | semantic entropy(值得时,贵 5-10x) |

- **独立性规则(决策#7,硬门)**: bearing claim 若 **<2 个独立模型家族**可用 → 强制 `Uncertain(SingleFamilyUnmet)` 或拦截,**绝不静默打 verified 徽章**;`independence` 字段写入 VerdictRecord 并在 Trust-UX 前置可见。
- **廉价优先级联**: 先跑确定性检查,被推翻则早停(不是跳过校验);Anthropic prompt 缓存(系统提示/工具定义/共享语料跨舰队缓存)+ Batch API 跑非交互校验。
- **失败**: 校验器分歧且确定性无法裁决 → 进阶段10;超成本上限 → 拒绝新调用(非记日志),未解决项留 `Uncertain`(一等终态)。

### 阶段 10 · 裁决 + 校准(conformal)
- **In**: 分歧 `VerdictRecord` 集
- **Out**: 终态 `VerdictRecord { calibrated_confidence }`
- **组件**: 裁决器**按证据不按投票**(AgentAuditor 思路,against-consensus,让正确少数击败编造多数,设计对齐 组件10);校准模块对 held-out 标签校准、报 ECE、**conformal 弃权阈值**(设计对齐 组件11);压制模型自报置信(verbalized confidence ECE 常 0.30+)。
- **Asking 接入(评估-Asking §1)**: 终态 `Uncertain` **且** bearing → 触发**校验门控的预算化升级**(每任务小打扰预算,异步批处理,机器可执行带 provenance);**不由自报置信触发**。预算用尽→`Uncertain` 作合法终态留存。
- **失败**: 裁决无法用证据化解 → 强制人工检查点(设计对齐 §2 人工检查点条件1)。

### 阶段 11 · claim 账本结果 UI + provenance 回链 + 完整性成绩单
- **In**: 终态 `Vec<VerdictRecord>` + `Vec<QuarantineEvent>` + `Vec<UsageRecord>`
- **Out**: 渲染态(claim 中心视图 + 护盾面板 + 成绩单)
- **组件(桌面端 MUST,moat-demo-first 排序 决策#2)**:
  - **claim 中心视图**: 每条 claim 是被告(5 态 verdict + 证据回链 + 信任徽章);点 claim 跳源 span。
  - **抗投毒护盾面板**: "扫描 N 源,隔离 X(隐藏提示/撤稿),消解 Y 工具注入"——一图可分享(威胁模型 §2,**演示叙事锚点**而非综述本身,设计核验报告 §4)。
  - **完整性成绩单**: 哈希链 provenance,每数字可重跑;conformal 弃权阈值做成 claim 视图可见徽章。
  - **不设单一绿色"全部通过"印章**(手与眼 §3.4):展示红队最强反论 + 覆盖盲区(流水线没查的东西)。

### 阶段 12 · provenance 门控文档 emit + RO-Crate + venue/jurisdiction 披露
- **In**: 终态 `Vec<VerdictRecord>`(只读 durable.db 按 claim_id)
- **Out**: 交付文档 + `ro-crate-metadata.json` + PROV-O RDF + 披露块
- **组件(手与眼 §3 唯一磁盘写入者)**:
  - **硬不变式**: 无未校验 claim 可渲染;每可渲染句/数/图带 `claim_id`;`Verified`→渲染并回链证据;`Uncertain`→显式标不确定;`Refuted`/`Quarantined`→排除;触及隔离源的 claim **doc-gen 硬拦**(除非显式记录覆盖,威胁模型 §1)。
  - **新合成 claim 重入校验**(`Synthesized` class):emit 前过蕴含/引文精度,不铸造无支撑连接句。
  - **RO-Crate/PROV-O 导出**(FAIR 零锁定)+ BagIt/OCFL 校验。
  - **披露引擎**: 按 venue+jurisdiction profile(ICMJE-2026 第V节 + 中国标识《办法》显隐式标签,但**作为能力非法律强制声称**,桌面端附录);EU AI Act Art.50 宽限窗口至 **2026-12-02**(决策#14)。
- **失败**: 缺可解析 claim_id 的内容被**拒绝**(非静默丢弃);RTBF 触发 → 源字节 crypto-shred + verbatim_text null 化但保 BLAKE3 保链有效 + verdict 转 `EvidenceRedacted` + forget 传播审计事件(决策#12)。

---

## C. P1 验收指标(三项,定义 + 产生方式 + 测量管线)

> 这三项是 P1 的注册门(架构与选型 §9 P1 验收:"文献场景过评测门槛");它们由**评测/校准 harness**(独立 build-blocker,本节只定文献垂直所需的产出侧)产生。所有指标对**固定、防泄漏、版本锁定**的标注集计算。

### 指标 1 · 召回率 (Recall)
- **定义**: 对一组带"已知相关文献金标准集"的查询,阶段3-4 检索 + 阶段6 摄入后,被正确纳入证据库的相关文献占金标准的比例。
- **产生**: 构建文献垂直评测集 = 一组研究问题 + 人工标注的相关文献全集(对齐科研基准哲学,非通用 agent 分数,设计对齐 §点1)。
- **测量**: `recall = |检索命中 ∩ 金标准| / |金标准|`,在阶段4 输出与阶段6 admitted 两点分别测(区分"没检索到" vs "被门挡掉")。漂移触发再校准(模型/版本升级破坏旧校准)。

### 指标 2 · 引文忠实度 (Citation Faithfulness)
- **定义**: 被 `Verified` 的 citation/descriptive claim 中,其被引源**确实蕴含**该 claim 的比例(ALCE 引文精度/召回 + NLI 接地)。
- **产生**: 阶段9 的逐 claim NLI(MiniCheck)结果 + 阶段10 校准,对照人工"源是否支撑"标注。
- **测量**: `faithfulness = |源真支撑的 verified claim| / |全部 verified citation claim|`;同时报 ECE(校准质量)。低于阈值不得注册该垂直(架构与选型 §9)。

### 指标 3 · 虚构引用率 (Fabricated-Citation-Rate)
- **定义**: 最终输出文档中,DOI 不可解析 / 源不存在 / 源不支撑的引用占全部引用的比例。**这是产品存亡指标**(LLM 基线 ~20% GPT-4o 引用全编造、~45% 真样式有错,设计对齐 附录A#5)。
- **产生**: 阶段5 DOI 解析硬门 + 阶段6 摄入门 + 阶段12 emit 门三道拦截后的残余;对照 CiteCheck 三分类(精确/轻损/编造)。
- **测量**: `fab_rate = |不可解析或不支撑的引用| / |输出全部引用|`;目标趋零,作为护盾面板与营销的可验证克制声明("非 injection-proof,但可审计")。**与基线 LLM 的差距即产品价值证明。**

---

## D. 实现里程碑(本垂直内的有序构建,moat-demo-first)
1. 阶段3-5 检索+CAS+获取渠道标签(BYO key + Crossref/Unpaywall 默认 + 撤稿门)→ 可演示"诚实检索 + 盗版硬拦 + 撤稿拦截"。
2. 阶段6 摄入门(视觉一致性 + 隐藏提示 + DOI 解析)+ 隔离队列 → 可演示**护盾面板**(护城河第一关)。
3. 阶段8-9 拆解 + 分层校验(确定性全 claim,LLM 仅 bearing)+ 独立性硬门 → 可演示 claim 账本 + 5 态 verdict。
4. 阶段10-11 裁决/校准 + 完整性成绩单 + conformal 弃权徽章。
5. 阶段12 provenance 门控 emit + RO-Crate + 披露引擎。
6. 评测 harness 接金标准集,产出三项 P1 验收指标 → 过门即注册该垂直。

---

## 三、校验流水线接口 + 分层策略 + 编排 DAG/监督器

> SSOT §3 · 可照建规格(接口 + DAG schema + 策略规则)。
> 上游已锁:Tauri/Rust + Rig(rig-core)进程内核心 + Tokio 监督器;SQLite(SQLCipher)+ LanceDB 在 `.workbench/`;校验为中心、claim 为被告、verdict 5 态、provenance 哈希链。
> 本节把"校验引擎 13 组件 / 分层策略 / 编排 DAG / 监督器 / Asking 共享 checkpointer"全部落成接口与规则。所有类型名对齐 §私有DB 的 durable schema。
> 命名约定:`snake_case` = 持久 schema 字段(durable.db);`PascalCase` = 进程内 Rust trait / 结构;`UPPER` = 枚举值。

---

## (A) 校验引擎 13 组件 —— 具体接口(input/output 类型)

### 跨组件公共类型(对齐 durable schema)

```rust
// 引用 durable.db 既有实体,不重复定义其列;这里给运行时句柄类型
type ClaimId = Ulid;            // claim.id
type SourceId = Ulid;           // source.id  (source.blake3_hash 是内容寻址锚)
type EvidenceSpanId = Ulid;     // evidence_span.id (source_id + blake3 + byte_offset + verbatim_text + parse_trust)
type VerdictId = Ulid;          // verdict.id (不可变、版本化,revises_verdict_id 链)
type VerifierId = Ulid;         // verifier.id (kind=HUMAN|MODEL, model_id, version, prompt_hash)
type TaintLabel = bitflags;     // UNTRUSTED_INGEST | QUARANTINED_LINEAGE | USER_ASSERTED | TOOL_OUTPUT | PII | PHI

enum Verdict5 {                 // 锁定 5 态枚举(durable enum)+ 语言无关机器码
  VERIFIED, REFUTED, UNCERTAIN, QUARANTINED, INJECTION_NEUTRALIZED,
}
// 注:EVIDENCE_REDACTED 由 RTBF/许可下架产生,属 verdict 状态机的派生态(§私有DB §RTBF),不在生成路径产出,故不入校验器输出枚举。

struct CalibratedConfidence {   // 绝不来自模型自报;由校验结果导出
  point: f32,                   // 经 held-out 标签校准
  conformal_band: (f32, f32),   // 弃权阈值用
  abstain: bool,
}

struct ProvenanceRef {          // 每个组件输出都必须携带,写入哈希链事件日志
  inputs: Vec<EntityRef>,       // PROV used
  activity: ActivityKind,       // PROV activity
  agent: VerifierId,            // PROV agent
  emitted_event_seq: u64,       // event.seq (append-only 哈希链)
}

struct IndependenceTag {        // 跨厂商独立性(承重 claim 闸门用)
  model_families: BTreeSet<ModelFamily>,   // 去重后的"独立家族"数
  effective_independent_votes: u8,
  single_family: bool,
}
```

> 设计不变量(贯穿 13 组件):
> 1. 每个 verdict **必须** `verdict_uses_evidence` 至少一条 `evidence_span`(理由可对照源核对,挫败"理由伪造")。无证据的 verdict 非法。
> 2. 仅靠单一隔离/未审源支撑的 `VERIFIED` **自动降级**为 `UNCERTAIN`。
> 3. 置信只由 `CalibrationModule` 产出;任何组件读到模型自报置信只能用于**排序**,绝不入 verdict。
> 4. taint 单调传播:下游 claim 继承其最弱支撑源的 taint 与信任级。

---

**#1 ClaimDecomposer(claim 拆解器)**
```rust
trait ClaimDecomposer {
  fn decompose(&self, proposal: GenerationProposal, ctx: ProjectCtx)
    -> Vec<AtomicClaim>;   // FActScore 式;每个原子 claim 可独立核查
}
// in:  GenerationProposal { text, source_taint: TaintLabel, output_locale }  (生成=不可采信提案)
// out: AtomicClaim { id, text, claim_class, bearing: bool, span_in_proposal }
enum ClaimClass { NUMERIC_STAT, CITATION, CAUSAL, METHODOLOGICAL, DEFINITIONAL, NON_RETRIEVABLE }
// bearing 由 §B 的承重定义打标;claim_class 决定 §B 的确定性路由
```

**#2 EvidenceGrounder(证据接地/检索器)**
```rust
trait EvidenceGrounder {
  fn ground(&self, claim: &AtomicClaim, corpus: CorpusHandle)
    -> GroundingResult;
}
// 混合检索:FTS5 词法(cache) + LanceDB 语义(cache) over 已摄入语料
// out: GroundingResult {
//   spans: Vec<EvidenceSpanId>,        // 钉在不可变 source 字节+偏移
//   status: NO_EVIDENCE_FOUND | EVIDENCE_FOUND,  // 关键:区分"没检索到" vs 后续"被反驳"
//   ood_flag: bool,                    // 分布外(SciFact-Open: OOD F1 掉≥15)→ 降置信、可触发人审
// }
```

**#3 IngestGate(摄入门 —— 第一也最可卖的关卡,在任何 LLM 看到内容前)**
```rust
trait IngestGate {
  fn screen(&self, raw: RawArtifact) -> IngestDecision;  // 确定性检查器,见 #6 的子集在此前置
}
struct RawArtifact { bytes_blake3, mime, declared_source_channel, sniffed_locale }
enum IngestDecision {
  ADMIT { source_id, taint: TaintLabel },
  QUARANTINE { quarantine_event_id, rule_hit: Vec<RuleId>, reason },  // 永不进 claim 库
  HUMAN_REVIEW { forensic_flag: Vec<ForensicFlag> },                  // 取证软标记,转人工不自动拒
}
// 硬拒(无歧义):不存在的 DOI、已撤稿(Crossref update-to + Retraction Watch)
// 软标记转人审(取证):statcheck/GRIM 命中、PubPeer 关注、tortured-phrase、元数据异常
// 视觉一致性不变量:比对"LLM 可见字符流" vs "人类可见渲染"(白底白字/微字体/页外/cmap 重映射/隐藏 HTML 注释)→ QUARANTINE
// 低置信非拉丁/CJK 抽取(sourceLocale 轴)→ 自动 HUMAN_REVIEW
// 产出:写 quarantine_event 实体(源/触发规则/隔离非删) + 哈希链事件
// 获取渠道:declared_source_channel 必须记入审计(出版商API/OA-CC/上传/订阅/抓取);盗版/影子库默认硬拦
```

**#4 EntailmentCitationVerifier(蕴含/引文校验器)**
```rust
trait EntailmentCitationVerifier {
  fn verify(&self, claim: &AtomicClaim, spans: &[EvidenceSpanId]) -> EntailmentResult;
}
// 逐 claim NLI(MiniCheck / Bespoke-MiniCheck-7B,GPT-4 级接地、~1/400 成本)——确定性层先行
// ALCE 式引文精度/召回;"被引源是否真支撑该 claim"
// out: EntailmentResult { entailed: bool, citation_precision, citation_recall,
//                         unsupported_sentences: Vec<Span> }  // 不蕴含 → 触发 CEG/RARR 检索-重写或拒绝
```

**#5 AdversarialRedTeam(对抗红队校验器)** ——【分层:仅承重/结论 claim】
```rust
trait AdversarialRedTeam {
  fn refute(&self, claim: &AtomicClaim, evidence: &[EvidenceSpanId], independence: IndependenceTag)
    -> RefuterAttempt;     // 唯一任务:找反例/混杂/缺失假设/矛盾源(波普尔证伪)
}
// out: RefuterAttempt { refuted: bool, counterexamples, confounders, contradicting_sources,
//                       attempt_id }   // 写 refuter_attempt 实体
// 闸门:refute 失败(没推翻)才放行该 claim 为候选 VERIFIED
// 独立性:若 independence.single_family → 不得出 VERIFIED,emit UNCERTAIN(单家族,独立性不满足)
```

**#6 DeterministicCheckers(确定性/符号检查器 —— 对所有 claim,§B 的便宜可守层)**
```rust
trait DeterministicChecker {           // 每个具体检查器实现同一 trait,可热加载(规则 feed)
  fn id(&self) -> CheckerId;
  fn applies_to(&self, c: &AtomicClaim) -> bool;
  fn run(&self, c: &AtomicClaim, spans: &[EvidenceSpanId]) -> DeterministicResult;
}
enum DeterministicResult { PASS, FAIL { detail }, NOT_APPLICABLE, NEEDS_HUMAN { forensic } }
// 具体检查器集合(MCP 网关后的工具,沙箱+审批):
//  - statcheck: p 值重算 (FAIL→NEEDS_HUMAN, 有误报)
//  - GRIM/GRIMMER: 均值-SD 可行性 (n<100)
//  - DOI 解析: Crossref→DataCite→OpenAlex→S2 (不存在→FAIL 硬拒)
//  - retraction: Crossref update-to + Retraction Watch (撤稿→FAIL 硬拒;PubPeer→NEEDS_HUMAN)
//  - code-recompute: 沙箱重算数字/统计(重跑检验、自由度、单位、多重比较校正)——绝不让 LLM 肉眼算术
//  - Lean: lean-lsp-mcp + Mathlib 形式化证明校验
//  - image: ImageTwin/Proofig(限定范围 + 人在环)
// 离线降级(显式决策):Crossref/Retraction 等在线门离线时 → fail-closed,该 claim 标 UNCERTAIN 而非 fail-open 放行
```

**#7 CrossVendorFleet(跨模型/跨厂商校验舰队)** ——【分层:仅承重 claim;独立性是一等闸门】
```rust
trait CrossVendorFleet {
  fn cross_check(&self, claim: &AtomicClaim, evidence: &[EvidenceSpanId], roster: ModelRoster)
    -> CrossVendorResult;
}
// CoVe 式独立作答(不看草稿)+ 跨厂商家族 + agent 身份匿名化(削自偏好) → 解相关
// out: CrossVendorResult { per_family_verdicts, agreement, independence: IndependenceTag }
// 独立性闸门(硬规则,§B):effective_independent_votes < 2 → 强制 UNCERTAIN(single-family)
// 前沿推理/对抗成员回退 Opus 4.8(Fable 5 已暂停,不入 roster);roster 由能力矩阵按 status/sunset 过滤
```

**#8 PRM(过程奖励/步级校验器)**
```rust
trait ProcessRewardModel {
  fn score_steps(&self, derivation: ReasoningTrace) -> Vec<StepScore>;  // 抓"答对但推理错"
}
// 多步推导(数学/方法学)逐步打分;低分步触发 #5/#6 重查该步
```

**#9 HallucinationDetector(幻觉检测器 —— 不可检索 claim)**
```rust
trait HallucinationDetector {
  fn detect(&self, claim: &AtomicClaim) -> HalluResult;  // claim_class == NON_RETRIEVABLE 时
}
// 廉价默认 SelfCheckGPT-NLI(~92 AUC-PR);值得时升 semantic entropy / SEPs(贵 5–10x)——级联升级
```

**#10 Adjudicator(裁决器)**
```rust
trait Adjudicator {
  fn adjudicate(&self, claim: &AtomicClaim, all_results: VerifierBundle) -> Adjudication;
}
// 仅在关键分歧点触发;推理树审计(AgentAuditor 思路,against-consensus)
// 按证据裁决,绝不按投票(共享谱系→相关错误,confabulation consensus)——正确少数可击败编造多数
// out: Adjudication { resolved: bool, chosen_verdict: Verdict5, rationale_spans }  // 写 adjudication 实体
// resolved==false → 升级人工检查点(§D)
```

**#11 CalibrationModule(校准模块)**
```rust
trait CalibrationModule {
  fn calibrate(&self, claim: &AtomicClaim, bundle: VerifierBundle) -> CalibratedConfidence;
  fn report_ece(&self, window: TimeWindow) -> EceReport;
}
// 输入信号:独立校验器支持比例 / 接地强度 / 反驳成败 / 确定性检查通过 / 独立家族数
// 压制模型自报置信;对 held-out 标签校准;conformal 定弃权阈值(abstain=可见徽章)
// 模型/版本漂移触发再校准门(verdict 钉 model+version,换模型→对仍生效 verdict 触发再验)
```

**#12 McpGateway(MCP 网关 —— 工具调用唯一咽喉)**
```rust
trait McpGateway {
  fn invoke(&self, tool: ToolRef, args: TypedArgs, caller_taint: TaintLabel) -> ToolResult;
}
// 三区信任 Core/Curated/Community;逐用户 OAuth(不共享 token)、逐工具/角色白名单
// 工具描述按内容哈希钉死,运行时漂移告警(rug-pull);工具输出=带污点数据,绝不回规划器当指令
// 输入与工具输出双向 DLP/PII/密钥脱敏;全程哈希链 provenance
// 出口:钉死每 agent 可调端点;隔离上下文默认拒网络出口;净化自动渲染 markdown 图片(EchoLeak 教训)
// 所有 #6 确定性检查器都是它后面的沙箱工具
```

**#13 AuditStore(审计链存储)**
```rust
trait AuditStore {                         // = durable.db 的 append-only 哈希链事件日志
  fn append(&self, ev: Event) -> u64;      // event(seq, ts, actor, op, payload_json, prev_hash, entry_hash)
  fn project_state(&self, at: Option<u64>) -> StateSnapshot;  // 事件溯源:state 是 log 的投影
  fn replay_from(&self, checkpoint: u64) -> ReplayCursor;     // 顺带白送崩溃续跑
}
// entry_hash = H(prev_hash ‖ canonical(payload));链头定期外部锚定(tamper-evident 非 -proof)
// 持久化:拆解/证据片段/每个 verdict/反驳尝试/确定性日志/裁决/校准/人工决定 + quarantine_event + UsageRecord
// 既是可解释性界面,也是再校准数据集,也是 §D 的 checkpointer 底座
```

---

## (B) 分层校验策略 —— 显式规则(锁定 #1:替换"全程每 claim 全舰队")

**核心规则:确定性/符号检查器对所有 claim 跑(便宜、可守、有地面真值处质量更高);LLM 对抗舰队(#5 红队 + #7 跨厂商)只对承重/结论 claim 跑。**

**R1 — 承重(bearing)claim 定义**(决定是否上 LLM 舰队):满足任一即 `bearing=true`:
- `claim_class ∈ {CONCLUSION 句, CAUSAL}`(结论句/因果断言);
- 该 claim 喂入任何不可逆/发表动作(doc-gen emit、对外结论);
- 下游有 ≥1 个其他 claim 依赖它(图入度 ≥1 的支撑节点);
- `CalibrationModule` 初判低置信且影响面大;
- 摄入门取证 flag 关联。
否则 `bearing=false`(routine claim)。

**R2 — 路由表**

| claim 状况 | 跑哪些组件 | 不跑 |
|---|---|---|
| 所有 claim（含 routine） | #2 接地 → #4 蕴含/引文 → #6 全部适用确定性检查器 → #11 校准 | #5 #7 #10 |
| `bearing=true` | 上面全部 + #5 红队 + #7 跨厂商舰队 +（分歧时)#10 裁决 | — |
| `claim_class==NON_RETRIEVABLE` | #9 幻觉检测（级联 SelfCheck→SEPs)+ #11 | #2 #4 |
| 多步推导 | 追加 #8 PRM 逐步 | — |

**R3 — 廉价优先级联（效率排序,不是跳过校验）**:`#6 确定性 → #4 NLI → #5/#7 LLM 舰队 → #10 辩论`。任一前序**确定性推翻**(如 DOI 不存在、重算不符)→ **早停**,直接 `REFUTED`,不再花 LLM 预算。

**R4 — 跨厂商独立性闸门(硬规则)**:对 `bearing=true` claim,若已连 `effective_independent_votes < 2`(< 2 个独立模型家族,或仅本地单家族)→ **禁止** `VERIFIED`,强制 `UNCERTAIN(单家族,独立性不满足)` 或按部署策略拦截。独立性在 Trust-UX 一等前置显示;部署向导做连通性检查;离线档 verdict 带"独立性:单家族/本地"标注。

**R5 — 单源降级**:仅靠单一隔离/未审源支撑的候选 `VERIFIED` → 自动 `UNCERTAIN`。

**R6 — 跑前预估(锁定 #1 必须)**:提交任务即对全 claim 集按 R1/R2 静态展开,产出 `RunEstimate { n_claims, n_bearing, est_agents, est_tokens, est_cost_usd, est_wallclock_under_quota }`,在 token-bucket / OTPM 当前限额下给"~$X / ~N agents / ~T 小时",超 `cost_cap` 前需 over-cap 审批。

**R7 — verdict 状态机(终态)**:`VERIFIED | REFUTED | UNCERTAIN | QUARANTINED | INJECTION_NEUTRALIZED`;`UNCERTAIN` 是一等成功态、阻断下游不可逆动作;`QUARANTINED`/`INJECTION_NEUTRALIZED` 来自摄入门/网关防御;重判=插新 verdict 行 + `revises_verdict_id`,绝不静默把 `UNCERTAIN` 升 `VERIFIED`。

---

## (C) 编排层 —— 声明式 DAG/状态机 schema + 监督器执行

### C.1 DAG schema(typed node I/O,场景注册表 (b) Workflow 段产出)

```yaml
# 声明式控制平面(确定性、可重放);活动是幂等可重试的 LLM/工具调用
workflow:
  id: lit_review_v1
  output_locale: "{{project.outputLocale}}"   # 注入 agent 提示
  nodes:
    - id: query_expand        # plan
      kind: AGENT
      in:  { task: TaskSpec }
      out: { queries: [Query] }
    - id: retrieve            # act (fan-out over 多源)
      kind: FANOUT
      over: queries
      in:  { query: Query }
      out: { hits: [SourceHit] }
      activity: mcp_tool       # 经网关,BYO key 限速
    - id: dedup_rank
      kind: REDUCE
      in:  { hits: [[SourceHit]] }; out: { ranked: [SourceHit] }
    - id: ingest_gate          # 组件 #3,每源
      kind: FANOUT; over: ranked
      out: { admitted: [SourceId], quarantined: [QuarantineEventId] }
    - id: extract_propose
      kind: AGENT
      out: { proposals: [GenerationProposal] }   # 不可采信
    - id: decompose            # 组件 #1
      kind: MAP; out: { claims: [AtomicClaim] }
    - id: verify_claim         # 每 claim 动态展开校验子图(见 C.2)
      kind: FANOUT; over: claims
      expand: per_claim_verifier_subgraph     # 运行期按 R1/R2 展开
      out: { verdicts: [VerdictId] }
    - id: barrier_all_verified
      kind: BARRIER            # loop-until-done:未决 claim 回流或升级人工
      until: "all(claims.verdict != PENDING)"
    - id: emit                 # provenance 门控:任何触及隔离源的 claim 硬拦
      kind: AGENT; guard: provenance_gate
  edges: [...]                 # 显式 DAG;handoff 深度 ≤ 8-10
```

```rust
enum NodeKind { AGENT, FANOUT, REDUCE, MAP, BARRIER, HUMAN_CHECKPOINT }
struct NodeIO { in_type: TypeRef, out_type: TypeRef }   // 编译期类型检查节点连线
// 静态 fan-out:DAG 里写死的并行(retrieve over queries)
// 运行期展开 fan-out:verify_claim 按 R1/R2 在运行时为每个 claim 生成校验子图(承重→加红队/跨厂商节点)
```

### C.2 per-claim 校验子图(运行期展开)

```
              ┌─ #6 deterministic checkers (all) ─┐
claim ─ #2 ─ #4 ─┤                                   ├─ #11 calibrate ─ verdict
   (ground)(nli)  └─ [if bearing] #5 redteam ──┐     │
                     [if bearing] #7 fleet ─────┴ #10 adjudicate(if disagree)
   (early-stop on any deterministic REFUTE; R3)        │
                                                 [if unresolved/低置信承重] → HUMAN_CHECKPOINT (§D)
```

### C.3 监督器(Tokio)执行机制

```rust
struct Supervisor {
  concurrency: Semaphore,        // 有界并发,起步 8–32,按机器/配额调
  buckets: TokenBucketMap,       // 按 (provider, model, 限流维度) 镜像厂商桶;池大小 = min(ITPM/in, OTPM/out)*0.7~0.8;OTPM 通常先撞顶
  cost_cap: HardCostCap,         // 两层:厂商 spend cap + 应用侧实时累加器;预计超预算→拒绝(不只记日志)
  idempotency: IdemStore,        // 每副作用/LLM 步一个幂等键 + 去重
  dlq: DeadLetterQueue,          // N 次重试后入 DLQ,其余继续跑
  event_log: AuditStore,         // 单一事实源;崩溃从最近 checkpoint 确定性重放
}
```
- **准入限流**:worker 池前置共享准入门;每次调用前预估 in/out token,所有桶有余量才放行 → 避免 fan-out 429 风暴;也是注入引发"agent 爆炸"的安全爆炸半径限制器 + 文献 BYO key 限速。
- **错误分类**:可重试(429/5xx/网络 → 指数退避 1s×2 封顶 60s + 抖动 + 遵 retry-after)vs 不可重试(4xx/内容策略 → 快速失败)。
- **持久化/续跑(Temporal 式拆分)**:确定性可重放控制平面(图:fan-out/barrier/loop-until-done)+ 幂等可重试活动(每 LLM/工具调用);以事件日志为单一事实源,任务可中断、可在另一台机从 checkpoint 恢复。
- **聚合**:fan-out/fan-in 优于长串行链;dedup + judge/裁决面板 + completeness critic;**舰队多数算力花在证伪上**(校验 agent 才是主体,生成 agent 只是提案源)。
- **批处理杠杆**:非交互校验(语料打分、judge 面板、离线综合)走 Batch API;系统提示/工具定义/共享语料跨舰队走 Anthropic prompt 缓存(命中输入不计 ITPM、~0.1x)。

---

## (D) 工作流事件日志与 Asking(人机操控)共享 checkpointer

**唯一基座:LangGraph 式 `interrupt()` + `Command(resume=...)`,checkpointer = §C.3 的 `AuditStore` 哈希链事件日志(同一个,不另起)。**

```rust
enum InterruptKind {
  ASKING,        // 系统发起:承重 claim 终态 UNCERTAIN 且下游影响大
  ANNOTATION,    // 用户发起:同步操控运行中 agent,或异步批注已完成输出
}
struct Interrupt {
  kind: InterruptKind,
  anchor: AnchorRef,            // 具体 claim_id / 工具调用 / span(机器可执行、带 provenance)
  question_or_note: Text,
  taint: TaintLabel,           // ask/note 本身是不可信输入,带污点
  checkpoint_seq: u64,         // = event.seq;暂停期间只花存储
}
enum HitlResolution { APPROVE_REJECT, EDIT_STATE, INSPECT_TOOL_CALL, MULTITURN_INPUT }
```

**统一规则**:
1. **同一通道、同一基元**:Asking 与批注是同一双向通道的两个方向(谁先开口),共用中断/恢复 + 同一 4 种 HITL 模式 + 同一可检索记忆层 + 同一护栏顺序。
2. **触发(锁定)**:Asking **由校验结果触发**(终态 UNCERTAIN 且承重),**绝不由模型自报置信触发**;自报不确定最多用于排序候选。
3. **预算化**:每任务一个很小的"打扰预算";相关中断在调度器**聚合去重**(上百 agent 的"中断风暴")后批量呈现;预算用尽则 UNCERTAIN 作为合法终态留存。
4. **护栏顺序(双向同序)**:先诚信层(拒造假/p-hack/夸大,凌驾用户)→ 再注入/外泄校验(ask/note 走同套 taint)→ 即时记忆注入 + 重规划。**操控可重排序,但不能在无独立高摩擦确认下翻转 verdict 或停用校验器**;诚信规则不可越权。
5. **反馈路径(锁定)**:笔记/回答 = **可检索记忆(即时,原子+带标签+锚 span,按需取 2-3 条重规划)+ 离线可测再校准(长期,AutoLibra→评测→A/B→回归门控)**;**不是**接进调度器的在线学习(Goodhart/奖励过优化)。
6. **持久化**:每个 ask/note/resolution 写哈希链事件 + durable 的 notes/asks 表 + human_decision;异步回答恢复时对账(可能已过时);全程 provenance 支持回滚与通道操纵检测。

---

## 实施里程碑(moat-demo-first,锁定 #2 顺序)

1. **M1 确定性脊柱**:#3 摄入门(视觉一致性 + DOI/撤稿硬拒 + 隔离)→ #6 确定性检查器(statcheck/GRIM/DOI/retraction/code-recompute)→ #13 审计链 + quarantine_event/UsageRecord schema → #11 校准骨架。产出完整性成绩单 + 护盾面板可 demo。
2. **M2 claim 中心 + 蕴含**:#1 拆解 → #2 接地 → #4 NLI/引文 → claim 视图 + provenance 可视化 + R3 级联(确定性早停)。
3. **M3 分层 LLM 舰队**:#5 红队 + #7 跨厂商 + R4 独立性闸门 + #10 裁决 + R6 跑前预估;角色化路由(连 providers,/models 入能力矩阵,Fable 5 排除、回退 Opus 4.8)。
4. **M4 编排 + 监督器**:DAG schema + per-claim 运行期展开 + Tokio 监督器(令牌桶/cost-cap/幂等/DLQ/事件重放)+ barrier/loop-until-done。
5. **M5 Asking/批注通道**:共享 checkpointer + 预算化中断 + 聚合去重 + 记忆层注入 + 护栏顺序。
6. **(同一发布内延后)** computer-use 审批沙箱、本地 GPU 全栈、三 i18n 轴 UI、MCP 包管理市场。

---

## 四、开源/闭源边界 + P0 范围 + 里程碑 + 功能构建顺序

## 四、开源/闭源边界 + P0 范围 + 里程碑 + 功能构建顺序

> 本节是可照建的 SSOT(单一事实源)。所有跨文档矛盾按《设计核验报告.md》§1/§2 已修;所有"已锁定决策"按 MEMORY + 提示中 15 条 BAKE-IN 不再重开。
> 三条贯穿性纠偏(全节生效):① 分层校验(确定性检查器 = 全 claim;LLM 对抗舰队 = 仅承重/结论 claim);② Fable 5 已暂停,前沿推理/裁决回退 **Opus 4.8**,能力矩阵带 status/sunset;③ 抗投毒对外措辞 = poison-resistant/可审计,**绝不** injection-proof,且 **不引用 CaMeL 67%**(我们建的是确定性入口检查 + 污点/provenance 标签 + 人工审批,不是能力追踪解释器)。

---

### A. 开源/闭源边界(组件/crate 级划线)

**总原则(已锁 2026-06-29):** 开源 = 让"信我们的 verdict"可被独立审计的那部分(校验引擎 + 检测规则);专有 = 编排、UX、工具生态、托管便利。从第一天就在 **crate/repo 层** 切开,因为以后切既贵又不可逆。

#### A.1 开源核 vs 专有壳:逐组件归属表

> 13 个校验脊柱组件按《护城河》§6 决策规则("下一代模型强一倍时更有用还是更没用")+ "可审计性是否服务信任"双过滤归属。

| # | 组件 | 归属 | 理由 |
|---|---|---|---|
| 1 | claim 分解器(claim decomposer) | **开源** | 验证的输入定义;闭源则无法核验"被告 claim 划得对不对" |
| 2 | 摄入门规则引擎(ingest-gate:视觉一致性不变量 + 隐藏提示检测 + 取证) | **开源** | 抗投毒是头号差异点,黑箱与"可审计"品牌矛盾;社区红队放大器 |
| 3 | 蕴含级引文/来源验证器(entailment/citation verifier) | **开源** | 护城河第 2 类底层能力,需可复核"被引源是否真支撑 claim" |
| 4 | 确定性/符号检查器库(statcheck/GRIM/Benford/DOI-撤稿/代码重算/Lean) | **开源** | 护城河第 1 层(最可守);非 LLM、可逐位复现,必须可审计 |
| 5 | 校准 + conformal 弃权计算引擎(calibration core) | **开源**(见 A.3 straddler 裁定) | 覆盖保证的数学可被独立验证;先验数据集闭源 |
| 6 | provenance / PROV-O 关系模型 + 哈希链事件日志校验器 | **开源** | 监管链可信度来自可独立重放/验链 |
| 7 | verdict 5 态状态机 + i18n 命名空间(机器码) | **开源** | 语言无关机器码是互操作契约,须公开稳定 |
| 8 | 编排监督器(Tokio supervisor:并发/令牌桶/成本上限/幂等/DLQ/事件重放) | **专有** | 《护城河》§6 "骑模型别建"类;价值在工程便利非可审计性 |
| 9 | 声明式 DAG/状态机编排层 + 场景注册表 | **专有** | 产品形态/编排 IP;开源核通过稳定接口被它调用 |
| 10 | 全部桌面 UX(任务看板、claim 视图、护盾面板、Trust-UX) | **专有** | 体验层;护城河预算投在此 |
| 11 | MCP 工具生态 / 学科包 / code-to-MCP | **专有**(网关规则除外,见 A.3) | 工具生态捕获价值 |
| 12 | 模型接入层 provider 抽象 + 角色路由 + 能力矩阵 | **专有** | 编排便利;但 verdict 上报的"模型+版本+prompt_hash"字段格式公开 |
| 13 | 托管便利(动态更新服务器、可选文献中继、身份/审批服务) | **专有** | 运营服务 |
| — | 检测规则 feed(注入手法/撤稿源/统计不可能模式/图像取证签名) | **开源 + 社区贡献** | GitHub 式飞轮,私有数据零外泄;独立签名内容包 |

#### A.2 crate / repo 拓扑与依赖方向(单向,核不依赖壳)

```
开源 monorepo: workbench-verify/  (license: Apache-2.0,允许闭源捆绑 + 专利授权)
├── crate: verify-core        # claim 分解、verdict 5 态机、verifier trait
├── crate: verify-checkers    # 确定性检查器库(statcheck/GRIM/Benford/DOI/重算/Lean 适配)
├── crate: verify-ingest      # 摄入门:视觉一致性不变量 + 隐藏提示 + 取证 + 隔离规则
├── crate: verify-entail      # 蕴含级引文/来源验证
├── crate: verify-calibrate   # conformal/ECE 计算 + 弃权阈值(纯算法,不含私有先验数据)
├── crate: verify-provenance  # PROV-O 模型 + 哈希链事件日志 + 验链器
├── crate: verify-rules-fmt   # 检测规则包格式 + 签名校验(fail-safe)+ 热加载契约
└── repo: detection-rules/    # 社区规则 feed(数据,非代码;独立签名;CONTRIBUTING + 规则 schema + CI lint)

专有 repo: workbench-shell/   (闭源)
├── 依赖 ↑ 上述开源 crate(单向:壳 → 核)
├── crate: shell-orchestrator # Tokio supervisor + 声明式 DAG + 场景注册表
├── crate: shell-providers    # Rig 封装 + 角色路由 + 能力矩阵
├── crate: shell-storage      # DurableStore/VectorStore/EventLog trait 实现(SQLCipher/LanceDB/PG)
├── crate: shell-ui (Tauri)   # webview 前端 + 三端 UX
└── crate: shell-services     # 身份/审批 + 动态更新 + 可选中继(独立部署)
```

**依赖不变式(CI 强制):** 开源 crate **禁止** 依赖任何专有 crate;`verify-*` 之间允许内部依赖但不得引入 Rig/Tauri/网络出口(保持可审计、可单测、可被第三方独立编译跑红队)。专有壳通过 trait 注入(provider、storage、tool transport),核侧只见接口。

#### A.3 两个 straddler 的裁定

- **校准模块(calibration)→ 拆两半:** 算法(conformal 覆盖证明、ECE 计算、弃权阈值逻辑)进开源 `verify-calibrate`(其覆盖保证的可独立验证 = 护城河第 3 层的可信度来源);**私有专家先验数据集 + 防污染轮换留出集 + canary** 留专有(护城河第 2 层的真资产 = 未泄漏标签,不是算法)。划线 = "公式开源,数据闭源"。
- **MCP 网关(三区信任)→ 拆两半:** "工具描述/输出皆不可信"的**确定性规则**(哈希钉死、变更检测、参数 schema 校验、输出标污点不回规划器)进开源 `verify-ingest` 旁的 `verify-rules-fmt`(属抗投毒可审计面);**三区注册表 UX + 工具生态 + 包分发** 留专有壳。划线 = "信任判定规则开源,工具市场闭源"。

#### A.4 社区检测规则贡献路径

`detection-rules/` 独立 repo,规则是**数据不是代码**(YARA/JSON schema 描述:新 tortured-phrase、新撤稿源、新统计不可能模式、新图像取证签名、新注入手法)。贡献流程:PR → schema lint + 红队 harness 回归(规则不得抬高误报率超阈)→ 维护者签核 → 打进独立签名内容包 → 经"快速规则 feed"通道分钟级热推(与缓慢需公证的应用整包解耦)。规则包 **fail-safe**:签名不符则拒跑(而非静默降级保护)。

---

### B. P0 范围(地质地基)+ 最先要验的 P0 风险

#### B.1 P0 必须交付(地基,不含任何垂直/校验业务逻辑)

P0 = "同一会话能在 OpenAI/Anthropic 间切换、流式跑通含工具调用,数据落进加密本地库,密钥在 keychain,能点击更新,内测者能注册获批"。

| 模块 | P0 交付物 | 验收 |
|---|---|---|
| Tauri 壳 | Rust core + webview + `ipc::Channel<T>` 有序流式 + `tokio::sync::Mutex` 共享态 + 抬高 `RLIMIT_NOFILE` | 空壳跑通,token 突发不压垮渲染 |
| Rig provider 抽象 + 角色路由 | 内部规范模型(Anthropic 富块结构)→ OpenAI/Anthropic 适配器 + OpenAI 兼容兜底;**连 provider 不连模型版本**,经 `/models` 发现填能力矩阵;自动角色路由(reading/reasoning/routine-verify/adversarial/adjudicate/modality)质量优先默认 + 高级 override 钉 model→role | 同一会话切两家跑通含工具调用与流式;能力矩阵带 status/sunset(Fable 5 标暂停,回退 Opus 4.8) |
| 独立性前置(角色路由正确性属性) | "已连独立模型家族数"做成能力矩阵 + Trust-UX 一等前置;<2 独立家族时承重 claim 强制 `uncertain(单家族,独立性不满足)` 或拦截;部署向导加连通性检查 | 单 key 时不静默打 verified 徽章 |
| 存储层 | SQLite(SQLCipher,密钥取自 keychain)`durable.db` + `cache.db` 双文件按可重建性打标 + LanceDB 嵌入式 + CAS(BLAKE3)+ 审计 JSONL 镜像;`DurableStore`/`VectorStore`/`EventLog` trait(PG/pgvector 改配置不重写);**durable schema day-one 含**:verdict 5 态枚举、`quarantine_event` 实体、`UsageRecord` 实体、i18n 三轴字段(uiLocale/outputLocale/sourceLocale)、数据类声明字段 | 写入可经哈希链重放;切 PG 仅改配置 |
| 密钥 | keyring crate → Keychain/Credential Manager(LLM key + 文献 key + SQLCipher 库密钥);**绝不** store 明文/Stronghold | key 永不落明文 |
| 流式 | per-agent `Channel<T>` 流式 + 节流 | — |
| 最小身份/审批服务 | 极小独立服务:手机 OTP 注册 + 创始人审批队列,**永不碰研究数据**;**新增第 9 攻击面**(手机号 PII、验证码滥用、账号枚举);提供**本地预审模式**保本地优先承诺 | 注册→审批→准入跑通;研究数据零经过该服务 |
| 点击更新 | Tauri updater + minisign(与 OS 代码签名两层都要)+ 动态更新服务器(通道/灰度/强制/回滚)+ 独立签名"规则 feed"快速通道(热加载,不重启) | 灰度分桶 + 强制下限 + 规则分钟级热推 |
| 成本/用量地基 | 令牌桶限流器(按 provider/model/限流维度,镜像厂商桶)+ 硬成本上限(超预算拒绝非记录)+ `UsageRecord` 归一化计费写入 | 防 BYO key 被 429;超顶前问 |

> P0 **不含**:任何 claim 校验逻辑、文献检索、评测/红队 harness、场景注册表、computer-use、本地 GPU 栈、三 i18n 轴 UI 控件(schema 留槽,UI 首发只露 uiLocale + outputLocale 两开关 + 语言无关 verdict 机器码/颜色 token)。

#### B.2 最先要验的 P0 风险(承重未知,编码前先打掉)

**头号风险 = Rig 的 `cache_control` / 带签名 thinking 块回放保真度。** 这是整个 provider 抽象正确性的支点(《架构》§10 风险 2)。P0 第一周做**保真度验证 spike**:
- 验 Rig 能否原样存取并回放带签名 thinking 块(同模型多轮循环内不裁剪/改写);
- 验 `cache: true` hint 透传到 Anthropic `cache_control`(≤4 断点、前缀不变式、TTL),用 `cache_read_input_tokens` 确认命中;
- 验 `POST /v1/messages/count_tokens` 准确预估(令牌桶准入门依赖它)。
- **判据 / 逃生舱:** 任一项不足 → 按《架构》§4 计划补写薄适配器(已预留)。**绝不**用 Anthropic 的 OpenAI 兼容 `/v1/` 端点做生产路径(test-only、有损)。
- 次要先验:规则/解析器能否在运行 Rig/Tokio 进程内**不重启热加载**(否则"分钟级规则 feed"是假承诺)。

---

### C. 里程碑排序(破 4 个 build-blocker + 校验脊柱;13 项 MUST 全保留,moat-demo-first)

> 排序原则:先打地基(P0)→ 再用"文献垂直"做最小可 demo 护城河的切片(P1),P1 只引入它真正需要的编排子集 + 最小评测 harness + 最小红队 harness → 其余按"骑模型别建"延后。每个 P1+ 里程碑落地前,法务/数据不变量(获取渠道合法性、数据类硬路由、撤稿筛查)**从第一天设计进流程**,不做后补合规层。

#### C.1 里程碑序列

| 里程碑 | 目标 | 破哪个 blocker | 关键产出 |
|---|---|---|---|
| **P0 地基** | 见 B 节 | — | Tauri 壳 + Rig 角色路由 + SQLCipher/LanceDB + keyring + 流式 + 身份/审批 + 点击更新 + 令牌桶/成本上限;先验 Rig 回放保真 |
| **P1a 校验脊柱 + 文献摄入(护城河切片)** | claim 账本 + 确定性检查器 + 摄入门 + provenance,跑通文献检索→摄入→claim→verdict | ① 文献垂直(检索侧工作流)② 编排子集(P1 只需 fan-out/fan-in + barrier,不要全 DAG) | 检索侧工作流(查询扩展→多源检索→去重排序→全文抓取打**获取渠道**标签→段落抽取)接进现成 10 阶段校验流水线;确定性检查器全 claim 跑;摄入门(视觉一致性+隐藏提示+取证)产隔离;claim 5 态账本 UI |
| **P1b 评测 harness(最小)** | 产出 P1 验收指标 + ECE/conformal | ③ 评测/校准 harness | 文献场景基准适配器 + 防泄漏留出集 + 校准计算管线 + 漂移再校准触发(见 D.1) |
| **P1c 红队/抗投毒 harness(最小)** | 首发 CI 门 + 可复现 demo | ④ 红队/抗投毒 harness | 注入语料(公开 + 自建科研)+ 自适应攻击 + ASR 阈值 + 连规则 feed(见 D.2) |
| **P1d 分层 LLM 校验 + 独立性聚合** | 承重 claim 上小型相关性感知聚合器 | — | 承重/结论 claim 才上 LLM 对抗(2–3 个刻意去相关检查器,测相关性,低有效独立票时弃权);跑前给"~$X / ~N agents / 你限额下 ~T 小时"预估 |
| **P2 场景注册表 + RTBF/备份** | 4 段契约 + 数据分析/论文写作场景 + 法律必修 | — | 注册表"零核心改动"插件;crypto-shred CAS + null 逐字文本留 BLAKE3 保链 + `evidence_redacted` 态 + 按数据类保留 + forget 传播审计;备份/恢复 + SQLCipher 密钥托管/恢复 |
| **P3 编排引擎全量** | dynamic workflow 模式全 DAG | ④(完成编排层全量) | 声明式 DAG/状态机(节点 I/O 类型、barrier/loop、场景注册路径)+ 连 Asking 的 interrupt/resume checkpointer + 监督器全特性(幂等/DLQ/事件重放)+ 进度树可观测 UI |
| **P4 规模化 + 延后 MUST + sidecar** | 上百规模 + "骑模型别建"几项 | — | Batch API + prompt 缓存优化;computer-use 全审批沙箱、本地模型 GPU 探测+跨 OS GGUF、三 i18n 轴 UI 控件+verdict 专家签核、MCP 包管理三区市场、多模态阅读区;可选 Codex(科研编码)/Claude SDK(Anthropic 保真)sidecar(默认关、不在 fan-out 主路径) |

#### C.2 13 项桌面 MUST 落点(全保留,moat-demo-first)

| 桌面 MUST | 落点 | 排序理由 |
|---|---|---|
| 项目文件夹工作区 | P0 | 一切挂靠的空间单元;存储/数据类/i18n 都是每项目 |
| 异步任务队列 + 运行看板 | P0(壳)→ P1a(接校验) | 反聊天框主屏;Tokio 监督器的桌面面 |
| **确定性检查器结果 + 完整性成绩单**(提为显式 MUST) | **P1a** | 护城河第 1 层,demo 首屏资产("扫描 N 源,隔离 X 投毒/撤稿") |
| claim 中心结果视图(5 态 + 证据回链 + 信任徽章) | **P1a** | 产品灵魂;区别于 deep-research/chat-PDF 的关键屏 |
| 抗投毒护盾面板 + 完整性报告 | **P1a** | 可见卖点;摄入门事件在此变可读;EU AI Act 第 50 条标注 |
| provenance/审计可视化 | P1a | chain-of-custody 一等界面 |
| BYO 密钥管理(keyring)+ 部署向导 + 本地模型设置 | P0(密钥)→ 向导 P1 | 成本模型;BYO 上手摩擦一等关切 |
| 最小披露/导出(provenance 门控 emit + RO-Crate/PROV-O) | P1a(最小)→ P2(深期刊模板) | 无未校验 claim 入文档 |
| 鲁棒性/用量可见面(token/舰队/缓存节省/超顶) | P0(地基)→ P1d(舰队规模可视) | "能不能跑"的面,非省钱功能 |
| 人机操控通道 UI(Asking 队列 + 批注) | **P3**(随 interrupt/resume) | 依赖 checkpointer + 节点级中断聚合去重 |
| 审批/沙箱 UX | P1a(仅覆盖代码执行确定性检查器)→ P4(完整 computer-use 审批沙箱) | 首发只需一个受控动作,省掉几乎全部 macOS 专属权限工程,零护城河损失 |
| MCP 工具包管理(三区信任) | **P4** | 《护城河》§6 "骑模型别建"桶 |
| 三语轴控件入 UI | schema P0 / UI **P4** | 三轴 day-one 进 schema(防破坏性迁移);UI 控件 + verdict 专家签核 + eager 加载延后,首发 en+zh-Hans 单轴切换 + 语言无关机器码 |

> **跨 OS 沙箱(安全基底)** 按 OS 落:macOS App Sandbox/seatbelt + entitlements(P1a 起,覆盖代码执行)→ Windows AppContainer + job objects + WFP(P4 随完整审批沙箱);**per-OS 可见沙箱强度指示**。非 Linux 容器。

---

### D. 评测/校准 harness + 红队/抗投毒 harness(P1 最小可建规格)

#### D.1 评测/校准 harness(P1b)

承担 4 个承重角色:注册门(场景过阈才注册)、护城河第 2/3 层举证、P1 验收、产 ECE/conformal。私有DB 已给**输出侧** schema(校准历史表),这里建**生产侧**。

**架构(crate `eval-harness`,专有壳侧,调用开源 `verify-calibrate`):**
- **基准适配器接口:** 统一 `Benchmark` trait → `{cases: Vec<Case>, metric: Metric, gate: Threshold}`;文献场景首发 3 指标:检索**召回/精度**、**引文忠实度**(蕴含级:被引源真支撑 claim)、**虚构引用率**(citation existence)。
- **语料:** 文献场景用公开基准 + 自策展私有留出集;数据分析场景预留 ScienceAgentBench/DSBench 式适配器。
- **防泄漏(护城河第 2 层不衰减的 R&D):** 私有轮换留出集 + canary 字符串 + 泄漏检测(每发布周期跑赢公网训练污染);私有专家标签**不入开源 repo**。
- **校准计算管线:** 经 `verify-calibrate` 算 ECE + conformal 覆盖(黑箱 API 上只需可交换性);弃权阈值产出做成 **claim 视图可见徽章**(不只后端)。
- **漂移再校准触发:** verdict 钉模型+版本;换模型/版本后,对仍生效 verdict 触发再验证 + 校准再验证门(《剩余议程》§9)。
- **命名控制框架:** 用 NIST AI RMF + GenAI Profile(NIST-AI-600-1)命名,供机构买家举证。
- **pass/fail 门:** 场景**不过评测阈值不得注册**(《架构》§6 第 d 段契约);P1 验收 = 文献场景过召回/引文忠实度/虚构率三阈。

#### D.2 红队/抗投毒 harness(P1c)

首发 CI 门 + 营销声明("每次构建对照公开攻击基准 + 自适应语料")。抗投毒是头号差异点,这门既阻塞又是信誉风险。

**架构(crate `redteam-harness`,调用开源 `verify-ingest`/`verify-rules-fmt`):**
- **注入语料(版本化):** 公开套件 AgentDojo + InjecAgent + MCPTox **+ 自建科研论文注入语料**(复刻 2025-07 arXiv 白底白字/微字体/页外文本/cmap 重映射/隐藏 HTML 注释案例)。语料 day-one 版本化(否则"自适应"无基线)。
- **"自适应"在确定性 CI 里的操作定义:** 不只跑静态套件(前沿模型在静态套件近零分但自适应仍有效,**静态近零是误导**);CI 跑一组针对当前规则集**定制重写**的载荷变体(模板化生成,固定种子可复现),度量绕过当前 `verify-ingest` 规则的成功率。
- **ASR(攻击成功率)通过阈值:** 按攻击面分桶设阈(摄入门绕过率、verdict 翻转率、外泄出口触发率);任一桶超阈 = CI 红、阻塞发布。阈值随规则 feed 更新收紧。
- **测的危害(按威胁模型 8 攻击面):** ① verdict 颠覆(标 verified)② 数据外泄(EchoLeak 式 markdown 图片)③ provider/出口滥用(在审稿件路由外部 AI)④ 持久化(库内休眠投毒)⑤ 钱包 DoS(派 N agent 烧 key)⑥ 文档污染。**数据外泄与 provider 路由当一等危害**(《剩余议程》§1 法务发现)。
- **连规则 feed:** harness 检出的新手法 → 进 `detection-rules/` → 规则包 → 分钟级热推;社区 PR 的新规则**必须过此 harness 回归**(不得抬高误报率超阈)才合并。
- **休眠投毒回归:** 验 provenance 触发再评估(撤稿状态变更重开相关 claim)+ 规则 feed 驱动定期重扫(保护已在库语料,非只新摄入)。
- **诚实姿态(对外):** harness 度量"抬高攻击成本"非"免疫";对外只说 poison-resistant/可审计;按**最弱模型**假设(本地模型无内置防御),保障放系统级层不依赖任何 provider 内置;per-deployment "防御档"指示。
