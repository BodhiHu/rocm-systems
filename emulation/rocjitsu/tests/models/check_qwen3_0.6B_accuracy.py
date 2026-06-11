from transformers import AutoModelForCausalLM, AutoTokenizer
from transformers.modeling_outputs import BaseModelOutputWithPast
import torch
import torch.nn as nn
from collections import OrderedDict

model_name = "/data/models/Qwen3-0.6B"

# Load tokenizer
tokenizer = AutoTokenizer.from_pretrained(model_name)

# Load CUDA model (model under test)
print("Loading CUDA model...")
model_cuda = AutoModelForCausalLM.from_pretrained(
    model_name,
    torch_dtype="auto",
    device_map="cuda"
).eval()

# Load CPU model (golden reference)
print("Loading CPU model (golden reference)...")
model_cpu = AutoModelForCausalLM.from_pretrained(
    model_name,
    torch_dtype="auto",
    device_map="cpu"
).eval()

print("Preparing input...")
# Prepare input
prompt = "Hi."
messages = [{"role": "user", "content": prompt}]
text = tokenizer.apply_chat_template(
    messages,
    tokenize=False,
    add_generation_prompt=True,
    enable_thinking=False
)
model_inputs_cuda = tokenizer([text], return_tensors="pt").to("cuda")
model_inputs_cpu  = tokenizer([text], return_tensors="pt").to("cpu")

print("Registering hooks...")
# ── Hook infrastructure ──────────────────────────────────────────────────────
cpu_inputs  = OrderedDict()
cuda_inputs = OrderedDict()
cpu_outputs  = OrderedDict()
cuda_outputs = OrderedDict()

def make_hook(store: dict, name: str):
    def hook(module, input, kwargs, output):
        # Support tuple outputs (e.g. attention layers return (hidden, cache, ...))
        if isinstance(output, tuple):
            tensor = output[0]
        elif isinstance(output, BaseModelOutputWithPast):
            tensor = output.last_hidden_state
        else:
            tensor = output

        if name == "lm_head":
            t = tensor.detach()
            print(f">>>>> detach ok")
            t = t.float()
            print(f">>>>> float ok")
            print(f">>>>> tensor: shape={t.shape} dtype={t.dtype} device={t.device}")
            # print('\n', t, '\n')
            # t = t.cpu()
            # print(f">>>>> cpu ok")
            store[name] = t
        else:
            store[name] = tensor.detach().float().cpu()

        if tensor.device.type == "cuda":
            print(f">>> [cuda] layer out: {name}")
        elif tensor.device.type == "cpu":
            print(f">>> [ cpu] layer out: {name}")
        else:
            print(f">>> ERROR: Unexpected device {tensor.device.type} for layer {name}")

    return hook

def make_pre_hook(device, pre_store: dict, name: str):
    def pre_hook(module, input, kwargs):
        if device == "cuda":
            print(f">>> [cuda] layer inp: {name}")
        elif device == "cpu":
            print(f">>> [ cpu] layer inp: {name}")
        else:
            print(f">>> ERROR: Unexpected device {device}")

    return pre_hook

# Register hooks on every named module (skip containers that are just wrappers)
_SKIP_TYPES = (
    nn.ModuleList,
    nn.ModuleDict,
    type(model_cpu),           # top-level model itself
)

def register_hooks(model, store, pre_store):
    handles = []
    for name, module in model.named_modules():
        assert isinstance(module, nn.Module)

        if not name:                          # root module
            continue
        if isinstance(module, _SKIP_TYPES):
            continue
        handles.append(module.register_forward_pre_hook(
            make_pre_hook(model.device.type, pre_store, name),
            with_kwargs=True))
        handles.append(module.register_forward_hook(
            make_hook(store, name),
            with_kwargs=True))
    return handles

handles_cpu  = register_hooks(model_cpu,  cpu_outputs,  cpu_inputs)
handles_cuda = register_hooks(model_cuda, cuda_outputs, cuda_inputs)

# ── Forward pass ─────────────────────────────────────────────────────────────
print("Running forward passes...")
model_cuda.eval()
model_cpu.eval()

with torch.no_grad():
    print(">>> cuda model:\n", model_cuda)
    _ = model_cuda(**model_inputs_cuda)
    print(">>> cpu  model:\n", model_cpu)
    _ = model_cpu(**model_inputs_cpu)

# Remove hooks
for h in handles_cpu + handles_cuda:
    h.remove()

# ── Layer-by-layer comparison ─────────────────────────────────────────────────
print(f"\n{'Layer':<50} {'Shape':<20} {'MaxAbsErr':>16} {'CosSim':>16} {'Pass':>16}")
print("-" * 128)

ATOL = 1e-2          # absolute tolerance  (tune as needed)
COS_SIM_THRESH = 0.999

all_pass = True
common_layers = [k for k in cpu_outputs if k in cuda_outputs]

for name in common_layers:
    cpu_t  = cpu_outputs[name]
    cuda_t = cuda_outputs[name].cpu()

    if cpu_t.shape != cuda_t.shape:
        print(f"{name:<50} shape mismatch  cpu={cpu_t.shape} cuda={cuda_t.shape}")
        all_pass = False
        continue

    max_err  = (cpu_t - cuda_t).abs().max().item()

    # Cosine similarity over flattened tensors
    c = cpu_t.flatten().double()
    g = cuda_t.flatten().double()
    cos_sim = (c @ g / (c.norm() * g.norm() + 1e-12)).item()

    passed   = (max_err < ATOL) and (cos_sim > COS_SIM_THRESH)
    all_pass = all_pass and passed
    flag     = "✓" if passed else "✗ FAIL"

    print(f"{name:<50} {str(tuple(cpu_t.shape)):<20} {max_err:>16.3e} {cos_sim:>16.6f} {flag:>16}")

print("-" * 128)
print(f"\nOverall: {'ALL PASS ✓' if all_pass else 'SOME LAYERS FAILED ✗'}")
print(f"Layers compared: {len(common_layers)}")

only_cpu  = set(cpu_outputs)  - set(cuda_outputs)
only_cuda = set(cuda_outputs) - set(cpu_outputs)
if only_cpu:
    print(f"Layers only in CPU  : {only_cpu}")
if only_cuda:
    print(f"Layers only in CUDA : {only_cuda}")

# ── Optional: original generation (CUDA model) ───────────────────────────────
print("\nRunning generation...")
generated_ids = model_cuda.generate(**model_inputs_cuda, max_new_tokens=1)
output_ids = generated_ids[0][len(model_inputs_cuda.input_ids[0]):].tolist()
print(f"Generated token ids : {output_ids}")
print(f"Decoded             : {tokenizer.decode(output_ids, skip_special_tokens=True)}")