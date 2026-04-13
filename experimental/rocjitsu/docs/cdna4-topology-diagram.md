# CDNA4 GPU Topology — Node Graph

Node-graph representation of `configs/amdgpu_cdna4.json`.

## Legend

| Shape | Meaning |
|-------|---------|
| **Blue box** | Hardware node — tree edges point child → parent |
| **Edge label** (◀/▶ name : protocol) | Socket — labels the edge from component to connector |
| **Purple hexagon** | Connector node — a link pattern; `links` edge goes to owning component |
| **Trapezoid** (⚙) | Config options on the hardware node |

### Connector mask builtins

| Mask | Expansion |
|------|-----------|
| `cross(A, B)` | All `(a, b)` pairs — full cross-product of instances of A and B |
| `cross_no_self(A, B)` | Cross-product excluding `a == b` |
| `pairs(k, k+1)` | Adjacent pairs within the same container — `(0,1), (1,2), …` |

A connector's **context** component determines how many times the entire
pattern repeats.  When omitted, the context defaults to the nearest common
ancestor of `from` and `to`.

## Diagram

```mermaid
flowchart TD
    classDef hw fill:#1a365d,stroke:#2b6cb0,color:#bee3f8
    classDef conn fill:#44337a,stroke:#805ad5,color:#e9d8fd
    classDef cfg fill:#2d3748,stroke:#4a5568,color:#a0aec0

    SOC["SOC · soc"]:::hw
    VRAM["VRAM · gpu_memory · ×1"]:::hw
    IOD["IOD · iod · ×2"]:::hw
    iod_cfg[/"⚙ num_hbm_stacks = 4"/]:::cfg
    XCD["XCD · xcd · ×8"]:::hw
    CP["CP · command_processor · ×1"]:::hw
    L2["L2 · l2_cache · ×1"]:::hw
    SE["SE · shader_engine · ×4"]:::hw
    CU["CU · compute_unit · ×8"]:::hw
    cu_cfg[/"⚙ wf=10 sgpr=104 vgpr=256 lds=160KB"/]:::cfg

    C_DISP{{"DISPATCH · cross(SE,CU) · idx j×8+k · lat=1 wt=2"}}:::conn
    C_MEM{{"MEM REQ · cross(SE,CU) · idx j×8+k · lat=1 wt=10"}}:::conn
    C_L2IOD{{"L2→IOD · ctx:XCD · node i/4 idx i%4 · lat=1 wt=3"}}:::conn
    C_PEER{{"IOD PEER · cross_no_self(i,j) · lat=1 wt=1"}}:::conn
    C_ADJF{{"ADJ FWD · pairs(k,k+1) · lat=1 wt=2"}}:::conn
    C_ADJR{{"ADJ REV · pairs(k+1,k) · lat=1 wt=2"}}:::conn

    %% Tree: children --> parent
    VRAM --> SOC
    IOD --> SOC
    iod_cfg --> IOD
    XCD --> SOC
    CP --> XCD
    L2 --> XCD
    SE --> XCD
    CU --> SE
    cu_cfg --> CU

    %% Connectors are inputs to components (links)
    C_DISP -- links --> XCD
    C_MEM -- links --> XCD
    C_L2IOD -- links --> SOC
    C_PEER -- links --> SOC
    C_ADJF -- links --> SE
    C_ADJR -- links --> SE

    %% from and to: socket names label the edges
    CP -- "▶ req_[n] : dispatch" --> C_DISP
    CU -- "◀ cpl : dispatch" --> C_DISP
    CU -- "▶ req : memory" --> C_MEM
    L2 -- "◀ cpl_[n] : memory" --> C_MEM
    L2 -- "▶ req : memory" --> C_L2IOD
    IOD -- "◀ msc.cpl_[n] : memory" --> C_L2IOD
    IOD -- "▶ peer_req : memory" --> C_PEER
    IOD -- "◀ peer_cpl : memory" --> C_PEER
    CU -- "▶ adj_req : untyped" --> C_ADJF
    CU -- "◀ adj_cpl : untyped" --> C_ADJF
    CU -- "▶ adj_req_r : untyped" --> C_ADJR
    CU -- "◀ adj_cpl_r : untyped" --> C_ADJR
```

## Connection summary

| Connector | From | To | Mask | Context | Instances |
|-----------|------|----|------|---------|-----------|
| DISPATCH | `CP.req_[j×8+k]` | `CU.cpl` | `cross(SE, CU)` | XCD (implicit) | 8 XCDs × 4 SEs × 8 CUs = **256** |
| MEM REQUEST | `CU.req` | `L2.cpl_[j×8+k]` | `cross(SE, CU)` | XCD (implicit) | 8 × 4 × 8 = **256** |
| L2 → IOD | `L2.req` | `IOD.msc.cpl_[i%4]` | identity | XCD (explicit) | 8 XCDs = **8** |
| IOD PEER | `IOD.peer_req` | `IOD.peer_cpl` | `cross_no_self(i, j)` | SOC (implicit) | 2 × 1 = **2** |
| ADJ FORWARD | `CU.adj_req` | `CU[k+1].adj_cpl` | `pairs(k, k+1)` | SE (implicit) | 8 × 4 × 7 = **224** |
| ADJ REVERSE | `CU[k+1].adj_req_r` | `CU.adj_cpl_r` | `pairs(k+1, k)` | SE (implicit) | 8 × 4 × 7 = **224** |
