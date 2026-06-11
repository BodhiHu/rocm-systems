from transformers import AutoTokenizer, AutoModelForCausalLM

tokenizer = AutoTokenizer.from_pretrained("/data/models/gpt-neo-125m")
model = AutoModelForCausalLM.from_pretrained(
    "EleutherAI/gpt-neo-125m",
    device_map="cuda"
)

prompt = "Good Morning,"

inputs = tokenizer(prompt, return_tensors="pt").to(model.device)

outputs = model.generate(
    max_new_tokens=2,
    **inputs,
)

print(tokenizer.decode(outputs[0], skip_special_tokens=True))
