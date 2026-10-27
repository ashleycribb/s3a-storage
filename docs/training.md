# S3A Research Agent Training & Fine-Tuning Pipeline

This document describes the end-to-end training loop for fine-tuning autonomous scientific research models from sealed S3A execution trajectories.

```
+--------------------------------------------------------------------------------------------------+
|                                    S3A TRAINING LOOP ARCHITECTURE                                |
|                                                                                                  |
|   +-------------------+      +-----------------------+      +-------------------+                |
|   | S3A Sealed Runs   | ---> | Consent Filter        | ---> | Trajectory Export |                |
|   | (<root>/runs/*.json)     | (model_training only) |      | (ChatML/ShareGPT) |                |
|   +-------------------+      +-----------------------+      +-------------------+                |
|                                                                       |                          |
|                                                                       v                          |
|   +-------------------+      +-----------------------+      +-------------------+                |
|   | Ollama Modelfile  | <--- | GGUF Quantization     | <--- | Unsloth LoRA/QLoRA|                |
|   | (Modelfile)       |      | (Q4_K_M 16-bit->4-bit)|      | (PyTorch/HF)      |                |
|   +-------------------+      +-----------------------+      +-------------------+                |
|             |                                                                                    |
|             v                                                                                    |
|   +-------------------+      +-----------------------+      +-------------------+                |
|   | agents/<id>.toml  | ---> | Action Evals          | ---> | Shadow Mode       | ---> Promote   |
|   | (point to model)  |      | (s3a-harness eval)    |      | (>= 80% agreement)|      Model     |
|   +-------------------+      +-----------------------+      +-------------------+                |
+--------------------------------------------------------------------------------------------------+
```

---

## 1. Step 1: Export Consent-Filtered Trajectories

Every agent execution produces structured provenance records in `<sandbox>/runs/<run_id>.json`. Under the S3A Consent Architecture, data can only be utilized for model training if explicit consent was granted:

```bash
# Export trajectories in OpenAI ChatML tool-calling format
s3a-harness export-trajectories --format chatml --out data/trajectories.jsonl --purpose model_training

# Alternatively, export in ShareGPT conversational format
s3a-harness export-trajectories --format sharegpt --out data/trajectories_sharegpt.jsonl
```

The exporter strictly enforces:
1. **Consent Filtering**: Only trajectories with `model_training` in their `consent_purposes` are exported. All other runs are omitted and listed with a reason in `filtered_runs`. New runs have **no** consent by default; grant it explicitly per run (e.g. `s3a-harness run --consent model_training`, or `RunState::with_consent_from`). Grants and denials are recorded in the run's `consent_events`.
2. **Deterministic Tool Schema**: Turns contain system prompts, user goals, assistant tool-calls with input argument hashes, and tool result payloads.
3. **No Hallucinated Paths**: Every turn reflects actual verified executions and supervisory gate interventions.

---

## 2. Step 2: Fine-Tuning with Unsloth (QLoRA)

[Unsloth](https://github.com/unslothai/unsloth) delivers 2–5x faster fine-tuning with 70% reduced VRAM using 4-bit quantization (QLoRA).

Create `train_unsloth.py`:

```python
import torch
from unsloth import FastLanguageModel
from datasets import load_dataset
from trl import SFTTrainer
from transformers import TrainingArguments

# 1. Configuration & Model Selection
max_seq_length = 4096
dtype = None # Auto detection (Float16 or Bfloat16)
load_in_4bit = True # 4-bit QLoRA

model, tokenizer = FastLanguageModel.from_pretrained(
    model_name="meta-llama/Meta-Llama-3.1-8B-Instruct",
    max_seq_length=max_seq_length,
    dtype=dtype,
    load_in_4bit=load_in_4bit,
)

# 2. Add LoRA Adapters
model = FastLanguageModel.get_peft_model(
    model,
    r=16, # Rank
    target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
    lora_alpha=16,
    lora_dropout=0.0, # Optimized for Unsloth
    bias="none",
    use_gradient_checkpointing="unsloth",
    random_state=3407,
)

# 3. Load Exported S3A Trajectories
dataset = load_dataset("json", data_files="data/trajectories.jsonl", split="train")

# 4. Configure SFTTrainer
trainer = SFTTrainer(
    model=model,
    tokenizer=tokenizer,
    train_dataset=dataset,
    dataset_text_field="messages",
    max_seq_length=max_seq_length,
    dataset_num_proc=2,
    packing=False,
    args=TrainingArguments(
        per_device_train_batch_size=2,
        gradient_accumulation_steps=4,
        warmup_steps=10,
        max_steps=120,
        learning_rate=2e-4,
        fp16=not torch.cuda.is_bf16_supported(),
        bf16=torch.cuda.is_bf16_supported(),
        logging_steps=5,
        optim="adamw_8bit",
        weight_decay=0.01,
        lr_scheduler_type="cosine",
        seed=3407,
        output_dir="outputs",
    ),
)

# 5. Train and Save LoRA Adapter
trainer.train()
model.save_pretrained_merged("outputs/s3a_scholar_model", tokenizer, save_method="merged_16bit")
```

---

## 3. Step 3: Quantization to GGUF

Convert the merged 16-bit weights to quantized GGUF format for local deployment:

```bash
# Using llama.cpp conversion tools
git clone https://github.com/ggerganov/llama.cpp
cd llama.cpp

# Convert HuggingFace format to 16-bit GGUF
python convert_hf_to_gguf.py ../outputs/s3a_scholar_model --outfile ../outputs/s3a-scholar-f16.gguf

# Quantize to 4-bit medium (Q4_K_M)
./llama-quantize ../outputs/s3a-scholar-f16.gguf ../outputs/s3a-scholar-q4_k_m.gguf Q4_K_M
```

---

## 4. Step 4: Ollama Modelfile Packaging

Package the GGUF binary into a local Ollama model instance. Create `Modelfile`:

```dockerfile
FROM ./outputs/s3a-scholar-q4_k_m.gguf

TEMPLATE """{{ if .System }}<|start_header_id|>system<|end_header_id|>

{{ .System }}<|eot_id|>{{ end }}{{ if .Prompt }}<|start_header_id|>user<|end_header_id|>

{{ .Prompt }}<|eot_id|>{{ end }}<|start_header_id|>assistant<|end_header_id|>

{{ .Response }}<|eot_id|>"""

SYSTEM """You are an autonomous scientific research reviewer agent for the S3A Scholar Pack.
You maintain strict epistemic rigor and citation preflight validation.
Subjective breakthrough claims and human judgments MUST always be routed to ask_human."""

PARAMETER temperature 0.1
PARAMETER top_p 0.95
PARAMETER stop "<|eot_id|>"
PARAMETER stop "<|end_header_id|>"
```

Build the Ollama model:
```bash
ollama create s3a-scholar-v1 -f Modelfile
```

---

## 5. Step 5: Update Agent Configuration

Update `agents/scholar_reviewer.toml` to point at the new fine-tuned model:

```toml
[agent]
id = "scholar_reviewer"
name = "Scholar Reviewer Agent"
system_prompt = "You are an autonomous scientific research reviewer agent."
model = "s3a-scholar-v1"
escalation_model = "meta-llama/Meta-Llama-3.1-70B-Instruct"

[limits]
max_turns = 15
max_tool_calls = 30
timeout_secs = 300
token_budget = 32000

[decision_thresholds]
tool_selection = 0.70
trimming = 0.65
stuck_detection = 0.80
escalation = 0.75

[safe_defaults]
tool_selection = "preflight_check"
trimming = "head_tail_trim"
stuck_detection = true
escalation = false

[tools]
allowed = [
  "preflight_check",
  "openalex_lookup",
  "crossref_lookup",
  "retraction_watch_check",
  "s3a_query",
  "s3a_write",
  "ask_human"
]
```

---

## 6. Step 6: Verification & Promotion Gate

Before promoting a newly fine-tuned candidate model to production, it must pass two mandatory criteria:

### 1. Action-Asserted Evaluation Suite (`s3a-harness eval`)
Evaluations assert directly on **actions taken** (concrete tool selection, gate hits, `ask_human` calls), NOT unconstrained free text.

```bash
s3a-harness eval --agent scholar_reviewer --set evals/scholar_evals.jsonl
```

Acceptance requirement: **100% pass rate** on critical safety and supervisory gate evaluations.

### 2. Shadow Mode Agreement (`s3a-harness shadow-report`)
Run the candidate model concurrently in shadow mode against live execution runs:

```bash
s3a-harness run --config agents/scholar_reviewer.toml --shadow s3a-scholar-v1 --goal "Synthesize literature and perform preflight checks"
```

Verify shadow model decision agreement:

```bash
s3a-harness shadow-report
```

Acceptance requirement: **$\ge 80.0\%$ operational decision agreement rate** with the baseline model before deployment promotion.
