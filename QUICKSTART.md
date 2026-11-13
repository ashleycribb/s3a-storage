# S3A Quickstart: Zero to Columnar in 5 Minutes

S3A is a high-performance, crash-resilient columnar database and storage engine built on sector-aligned Hyper-Tiles.

Here is how to install, initialize a vault, import data, query it, and export results in under 10 commands.

---

### Step 1: Install S3A

Using Cargo:
```bash
cargo install s3a-cli
```

Or using the one-line installer:

**Linux & macOS:**
```bash
curl -fsSL https://raw.githubusercontent.com/ashleycribb/s3a-storage/main/install.sh | bash
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/ashleycribb/s3a-storage/main/install.ps1 | iex
```

Verify your installation:
```bash
s3a --version
```

---

### Step 2: Initialize a Vault (Command 1)

Create a fresh S3A vault (`./my.s3a` by default):
```bash
s3a init
```

---

### Step 3: Put and Get Key-Value Records (Commands 2 & 3)

Store structured documents or values:
```bash
s3a put user:101 '{"name":"Alice","role":"engineer","active":true}'
```

Retrieve stored records:
```bash
s3a get user:101
```

---

### Step 4: Import Data from CSV (Command 4)

Create a sample CSV file:
```bash
echo "timestamp,sensor_id,metric_id,value" > metrics.csv
echo "1000,42,1,98.6" >> metrics.csv
echo "1010,42,1,99.1" >> metrics.csv
echo "1020,42,1,97.4" >> metrics.csv
```

Import it directly into your vault (auto-detects format and columnar layouts):
```bash
s3a import metrics.csv
```

---

### Step 5: Query Records with S3A-QL (Command 5)

Execute native S3A-QL queries with formatted ASCII table output:
```bash
s3a query "SIFT TELEMETRY WHERE SENSOR_ID = 42"
```

Or with machine-readable JSON output:
```bash
s3a query "SIFT TELEMETRY WHERE SENSOR_ID = 42" --json
```

---

### Step 6: Export Data (Command 6)

Export vault contents back to CSV or JSON:
```bash
s3a export backup.csv
```

---

### Step 7: Inspect and Tour (Commands 7 & 8)

Check vault summary (file size, tile counts by type, generation, active slot):
```bash
s3a info
```

Take an architectural tour of the building floors and hyper-tiles:
```bash
s3a tour
```

---

### Try the Instant Demo Vault

Want to explore sample books, sensor telemetry devices, and system audit events without creating any files?
```bash
s3a demo
s3a ls --vault ./demo.s3a
s3a tour ./demo.s3a
```

---

### Getting Help

Every command includes built-in help and copy-paste examples:
```bash
s3a help
s3a help query
s3a help import
```
