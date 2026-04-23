# amdisa — AMD GPU ISA Code Generation Pipeline

Python library for parsing AMD Machine-Readable ISA (MR ISA) XML specifications
and generating C++ source files for the rocjitsu project.

## Modules

| Module | Purpose |
|---|---|
| `parser.py` | Parse MR ISA XML specs into `IsaSpec` objects |
| `gpuisa.py` | Core data structures (`IsaSpec`, `Instruction`, `InstEncoding`, `Operand`) |
| `isa_profile.py` | Per-ISA profile constants and encoding rules |
| `semantics.py` | Derive instruction semantics from mnemonics |
| `cross_isa.py` | Cross-ISA instruction overlap analysis |
| `codegen.py` | Generate C++ decoders, encoders, and instruction execute bodies |
| `legalization.py` | Generate cross-ISA legalization tables (Action classification) |
| `legalization_codegen.py` | Emit C++20 `LegEntry[]` legalization table files |
| `encoding_translator_codegen.py` | Emit C++20 neutral field structs + decode/encode functions using `machine_insts.h` typed structs |

## Prerequisites

```bash
pip install cgen  # required by codegen.py
```

## Usage

All commands are run from the rocjitsu project root with `PYTHONPATH` set:

```bash
export PYTHONPATH=$PWD/lib/python
export MRISA=/path/to/mrisa  # directory containing amdgpu_isa_*.xml files
```

### Regenerate ISA decoders/encoders (single ISA)

```bash
python3 -m amdisa --gen-all \
  -o lib/rocjitsu/src/rocjitsu/isa/arch/amdgpu \
  $MRISA/amdgpu_isa_cdna4.xml
```

### Regenerate ISA decoders/encoders (all 9 ISAs with shared execute templates)

```bash
python3 -m amdisa --multi \
  cdna1:$MRISA/amdgpu_isa_cdna1.xml \
  cdna2:$MRISA/amdgpu_isa_cdna2.xml \
  cdna3:$MRISA/amdgpu_isa_cdna3.xml \
  cdna4:$MRISA/amdgpu_isa_cdna4.xml \
  rdna1:$MRISA/amdgpu_isa_rdna1.xml \
  rdna2:$MRISA/amdgpu_isa_rdna2.xml \
  rdna3:$MRISA/amdgpu_isa_rdna3.xml \
  rdna3_5:$MRISA/amdgpu_isa_rdna3_5.xml \
  rdna4:$MRISA/amdgpu_isa_rdna4.xml \
  --gen-all --gen-shared-execute \
  -o lib/rocjitsu/src/rocjitsu/isa/arch/amdgpu
```

### Regenerate DBT legalization tables (all ISA pairs)

```bash
python3 -m amdisa --multi \
  cdna1:$MRISA/amdgpu_isa_cdna1.xml \
  cdna2:$MRISA/amdgpu_isa_cdna2.xml \
  cdna3:$MRISA/amdgpu_isa_cdna3.xml \
  cdna4:$MRISA/amdgpu_isa_cdna4.xml \
  rdna1:$MRISA/amdgpu_isa_rdna1.xml \
  rdna2:$MRISA/amdgpu_isa_rdna2.xml \
  rdna3:$MRISA/amdgpu_isa_rdna3.xml \
  rdna3_5:$MRISA/amdgpu_isa_rdna3_5.xml \
  rdna4:$MRISA/amdgpu_isa_rdna4.xml \
  --gen-legalization \
  --legalization-output lib/rocjitsu/src/rocjitsu/isa/dbt/generated
```

Generates `legalization_tables.h` (shared header) and one `legalize_<src>_<dst>.cpp`
per supported ISA pair (27 pairs). Each file contains a sorted `LegEntry[]` array
classifying every source instruction as Identity, Substitute, Lower, or Expand.

### Regenerate DBT encoding translator tables (per ISA pair)

```bash
python3 -m amdisa --multi \
  cdna1:$MRISA/amdgpu_isa_cdna1.xml \
  cdna2:$MRISA/amdgpu_isa_cdna2.xml \
  cdna3:$MRISA/amdgpu_isa_cdna3.xml \
  cdna4:$MRISA/amdgpu_isa_cdna4.xml \
  rdna1:$MRISA/amdgpu_isa_rdna1.xml \
  rdna2:$MRISA/amdgpu_isa_rdna2.xml \
  rdna3:$MRISA/amdgpu_isa_rdna3.xml \
  rdna3_5:$MRISA/amdgpu_isa_rdna3_5.xml \
  rdna4:$MRISA/amdgpu_isa_rdna4.xml \
  --gen-encoding-translators --encoding-pair "cdna4->rdna4" \
  --encoding-translator-output lib/rocjitsu/src/rocjitsu/isa/dbt/generated
```

Generates two files:
- `encoding_fields.h` — ISA-neutral field structs derived from ALL loaded ISAs
  (union of fields across all 9 ISAs for each encoding format)
- `encoding_<src>_to_<dst>.h` — decode functions (src struct → neutral fields),
  encode functions (neutral fields → dst struct), and dispatch composing both

Decode/encode functions use the existing `machine_insts.h` typed bitfield structs.
Adding a new ISA pair reuses existing decode/encode functions for shared ISAs.
Coherency remapping uses shared helpers in `isa/dbt/encoding_translator.h`.

### Format generated files

Always run after any codegen:

```bash
./scripts/clang_format.sh lib/rocjitsu/src/rocjitsu/isa/dbt/
```

## Generated file locations

| Generated files | Location | Generator |
|---|---|---|
| ISA decoders, encoders, instruction bodies | `isa/arch/amdgpu/<isa>/` | `codegen.py` |
| Cross-ISA legalization tables | `isa/dbt/generated/` | `legalization_codegen.py` |
| Encoding decode/encode functions | `isa/dbt/generated/` | `encoding_translator_codegen.py` |

The encoding translator engine (`isa/dbt/encoding_translator.h`) is hand-written
and shared across all ISA pairs. Only the per-pair decode/encode functions and neutral field structs are auto-generated.
