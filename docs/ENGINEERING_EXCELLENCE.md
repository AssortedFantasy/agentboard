# Engineering Excellence in Agentboard

Engineering excellence means that every facet of the project has been considered with multifaceted details and engineering tradeoffs in mind. When someone uses Agentboard or reads its source, they should feel that deep care was put into the low-level details. This prevents the project from devolving into "slop" over time.

These guidelines are not a roadmap, but rather core principles and considerations to keep top-of-mind while building and maintaining Agentboard.

---

### 1. High-Quality, Modern Testing
Tests must be genuinely good by current engineering meta-standards. This doesn't just mean high line coverage; it means meaningful coverage.
- **End-to-End & Integration**: Test the CLI against a real SQLite database exactly as an agent would use it.
- **Concurrency Testing**: Ensure that multiple connections attempting to claim tasks or modify the same records do not cause deadlocks or corrupt data.
- **Resilience to Failure**: Tests should confirm that bad inputs or unexpected states fail gracefully and deterministically.

### 2. Database Efficiency (Space and Speed)
The SQLite schema must be designed for performance and space efficiency.
- **Normalization**: Avoid storing redundant data unnecessarily.
- **Indexing**: Lookups should be fast. Ensure foreign keys and frequently queried columns (like status, owner, tags) are properly indexed.
- **Query Optimization**: Keep the database lean so that complex aggregations or full-text searches do not bog down execution time.

### 3. Discoverability & Command Organization
An AI agent with a fresh context window should not need dozens of tool calls to understand how to use Agentboard.
- **Logical Hierarchy**: CLI commands should follow a predictable verb-noun or noun-verb structure that agents can easily guess.
- **Crux / Summary Views**: Provide high-level "skill" commands or summary outputs that give an agent the crux of the workspace's state immediately. Help commands should be dense and informative.

### 4. Context & Token Economics
We are optimizing for AI model usage. Every token matters.
- **Hop Efficiency**: Minimize the number of sequential tool calls an agent must make to achieve a goal.
- **Input/Output Token Optimization**: Responses should be concise. Exclude decorative borders, chatty text, or massive JSON dumps unless explicitly requested. Return only the data necessary to make the next decision.

### 5. Thoughtful Defaults
Defaults should be chosen with immense care.
- **The 90% Rule**: The default behavior of a command should be exactly what an agent wants 90% of the time, requiring zero configuration flags.
- **Intelligent Truncation**: Default limits (e.g., max rows, max output size) should be implemented out-of-the-box to prevent accidental context window blowouts.

### 6. Actionable Errors
When an agent makes a mistake, the error message should instantly teach them how to fix it.
- **No Dead Ends**: "Invalid argument" is a bad error. "Error: ID must be an integer. Usage: agentboard post show <id>" is a good error. Actionable errors prevent wasted AI reasoning loops.

### 7. Idempotency & Retry Safety
Agents operate in environments where they might time out, crash, or get confused.
- **Safe Retries**: Operations should ideally be idempotent where practical. If an agent tries to apply the same tag twice, or claim a task they already claimed, it should succeed or gracefully no-op without corrupting state.

### 8. Strict Concurrency (ACID)
Agentboard assumes local operation, but multiple independent agents might hit the database concurrently.
- **Transactions**: Use SQLite transactions properly. An agent claiming a task must be an atomic operation so two agents never think they both own the same work.

### 9. Performance & Latency (CLI Startup)
Agentboard is invoked repeatedly via CLI.
- **Lean Binaries**: Every millisecond the CLI takes to boot is wasted AI time. The Rust binary must stay lean and fast. Avoid heavy dependencies that slow down startup time.

### 10. CLI API Stability
Agents will essentially "hardcode" command structures into their prompts, scripts, or operational habits.
- **Rigid Contracts**: Treat the CLI arguments and standard output formats (especially JSON/JSONL modes) as a rigid API contract. Do not break compatibility lightly.

### 11. Pushing Compute to the Database
AI model compute is expensive and slow; SQLite compute is cheap and fast.
- **Leverage SQL**: Push filtering, joining, sorting, and aggregations down to the database level. Do not make the agent retrieve a list of 1000 posts just to filter for the 5 that are "unresolved". Provide a command that lets SQLite do the work.