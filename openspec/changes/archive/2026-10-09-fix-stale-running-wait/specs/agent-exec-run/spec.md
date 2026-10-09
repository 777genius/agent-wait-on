## REMOVED Requirements

### Requirement: wait サブコマンドの待機期限オプション

**Reason**: Canonical agent-exec-run carries three duplicate copies of this requirement; removing every copy lets the consolidated requirement below be promoted exactly once.
**Migration**: The ADDED requirement of the same name re-adds the complete consolidated normative text and scenario union (deadline/poll, progress hints and stale running observation); no behavior is dropped.

## ADDED Requirements

### Requirement: wait サブコマンドの待機期限オプション

`wait` サブコマンドは既定では最大 30 秒までジョブの終端状態を待機しなければならない（MUST）。待機上限は `--until <seconds>` によって上書きできなければならない（MUST）。`--forever` が指定された場合は終端状態になるまで無制限に待機しなければならない（MUST）。ただし、下記の read-only observation inconsistency 条件成立時は job を終端化せず、wait 自体を型付きエラーで終了しなければならない（MUST）。`--until` と `--forever` は互いに同時指定を許可してはならない（MUST NOT）。

待機上限に達してもジョブは終了させてはならない（MUST NOT）。終端状態まで到達した場合は `state` と `exit_code` を返さなければならない（MUST）。待機上限に達してもジョブが非終端状態の場合は非終端の `state` を返し、`exit_code` を含めてはならない（MUST NOT）。

`wait` のレスポンスは、state.json が読める場合、`stdout_total_bytes`・`stderr_total_bytes`・`updated_at` を必ず含めなければならない（MUST）。これは満期到達時・終端到達時の両方に適用される（MUST）。state.json がレース条件で未作成の場合はこれらを省略してよい（MAY）。

待機期限指定は秒単位の `--until` に統一しなければならない（MUST）。`--timeout-ms` は有効なオプションとして受け付けてはならない（MUST NOT）。

`wait` のポーリング間隔は秒単位の `--poll <seconds>` で指定できなければならない（MUST）。この間隔は観測用の近似値であり、ミリ秒精度の厳密なチェック時刻を保証してはならない（MUST NOT）。

#### Scenario: wait uses the default 30 second deadline

Given a running job created by `agent-exec run -- sh -c "sleep 1; echo done"`
When `agent-exec wait <job_id>` is executed
Then the wait returns within approximately 30 seconds
And if the job finished within the deadline, the response state is terminal

#### Scenario: wait --until returns while the job keeps running

Given a running job created by `agent-exec run -- sh -c "sleep 10"`
When `agent-exec wait --until 1 <job_id>` is executed
Then the response state is `created` or `running`
And `exit_code` is absent

#### Scenario: wait --forever preserves unbounded waiting

Given a running job created by `agent-exec run -- sh -c "sleep 1; echo done"`
When `agent-exec wait --forever <job_id>` is executed
Then the response state is terminal after the job exits

#### Scenario: wait --until and --forever are mutually exclusive

Given a user executes `agent-exec wait --until 1 --forever <job_id>`
When clap validates arguments
Then the command fails with usage error

#### Scenario: wait exposes second-based poll option

Given a user inspects `agent-exec wait --help`
When the polling option is shown
Then the canonical polling option is documented in seconds
And the help text does not imply millisecond-accurate checking

#### Scenario: wait rejects removed timeout-ms spelling

Given a user executes `agent-exec wait --timeout-ms 100 <job_id>`
When clap validates arguments
Then the command fails with usage error
And stdout is empty

#### Scenario: wait timeout returns progress hints

**Given**: a running job created by `agent-exec run -- sh -c "sleep 10"`
**When**: `agent-exec wait --until 1 <job_id>` is executed
**Then**: the response includes `stdout_total_bytes`, `stderr_total_bytes`, and `updated_at`
**And**: `exit_code` is absent

#### Scenario: wait terminal response also returns progress hints

**Given**: a finished job
**When**: `agent-exec wait <job_id>` returns its terminal response
**Then**: the response includes `stdout_total_bytes`, `stderr_total_bytes`, and `updated_at`
**And**: `exit_code` is present

The shared wait implementation MUST apply the exact conservative probe, monotonic streak, final-read precedence and typed envelope specified here: only explicit Unix ESRCH for a valid recorded positive PID in running state may start a streak; at least three consecutive samples spanning at least 10 monotonic seconds are required. Unknown/present, changed (pid, started_at), created and terminal reset it. Finite deadlines take precedence and are never extended. Terminal states retain normal log-drain semantics. Before error, re-read and re-probe the same running identity. No job file or result may be changed and no termination signal may be sent. Non-Unix and pid:null retain prior behavior. PID reuse observed present is not absence.

A confirmed observation inconsistency MUST return ok=false, type=error, error.code=job_observation_inconsistent, retryable=true with details job_id, absolute root, state=running, pid and process_observation=absent, with no fabricated exit_code/signal/finish time. CLI exits 1; MCP returns the domain envelope; HTTP wait returns 409. Positional and exact-reference binding MUST remain unchanged. A later saved terminal result MUST remain recoverable through a fresh status/wait. This is a wait observation failure, not proof of job failure or process exit cause.

#### Scenario: stale running observation ends wait only
**Given**: running state has a valid recorded PID with explicit absence sustained across the threshold
**When**: a CLI, root-bound CLI, MCP or HTTP wait observes the same inconsistency at final read
**Then**: wait returns the typed observation error and leaves every job byte unchanged
**And**: no workload is cancelled and no job result is invented

#### Scenario: terminal persistence and finite budget win
**Given**: a waiter is tracking absence
**When**: its finite budget expires or a terminal save is observed before error
**Then**: it follows the existing budget or terminal/log-drain contract instead
**And**: after an error a later terminal save remains available to a new wait

#### Scenario: conservative unknown and identity reset
**Given**: an absence streak exists
**When**: observation is unknown or present, or pid/started_at changes, or state becomes created/terminal
**Then**: the streak resets and is not resumed from job age
**And**: permissions, invalid PID and PID reuse do not fabricate an absence
