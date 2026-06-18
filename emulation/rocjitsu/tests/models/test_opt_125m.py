from transformers import AutoTokenizer, AutoModelForCausalLM

tokenizer = AutoTokenizer.from_pretrained("/data/models/opt-125m")
model = AutoModelForCausalLM.from_pretrained(
    "/data/models/opt-125m",
    device_map="cuda"
)

prompt = "Good morning,"

inputs = tokenizer(prompt, return_tensors="pt").to(model.device)

outputs = model.generate(
    max_new_tokens=2,
    **inputs,
)

print(tokenizer.decode(outputs[0], skip_special_tokens=True))

