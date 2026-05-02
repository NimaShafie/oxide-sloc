# Jenkins job bootstrap

## Operator workflow (overview)

1. `cp ci/jenkins/.env.example ci/jenkins/.env` — fill in `JENKINS_TOKEN`
2. _(Recommended)_ Pre-populate the agent rust-cache: run `bash ci/jenkins/install-rust-cache.sh` or build the Docker agent image from `ci/jenkins/Dockerfile.agent`. On air-gapped agents this step is required; on network-connected agents Rust downloads at runtime.
3. `set -a; source ci/jenkins/.env; set +a && bash ci/jenkins/preflight.sh` — all checks must pass
4. Run the `createItem` curl (Step 1 below)
5. Run the seed-build curl (Step 2 below)

On a network-connected agent, step 2 is optional.

---

## Obtaining credentials

### Initial admin password

If Jenkins was just installed and has never been unlocked:

**Native install (systemd / RPM / DEB package):**
```bash
sudo cat /var/lib/jenkins/secrets/initialAdminPassword
# some installs use /var/jenkins_home instead:
sudo cat /var/jenkins_home/secrets/initialAdminPassword
```

**Docker install:**
```bash
docker exec <container-name-or-id> cat /var/jenkins_home/secrets/initialAdminPassword
```

Paste this password into the Jenkins setup wizard at `http://<host>:8080/`.

#### If initialAdminPassword is gone

After first-run setup completes, Jenkins deletes that file. If you've also lost your token (e.g. you re-cloned and `ci/jenkins/.env` is gone), use one of:

- **Browser:** sign in at `http://<host>:8080/` with your admin password → click your username (top-right) → **Configure** → scroll to **API Token** → **Add new Token**. (The direct URL `/user/<id>/configure` is gone in some recent LTS builds; use the dropdown link instead.)
- **REST (Path B below):** the same admin password works — no `initialAdminPassword` needed.
- **Reset admin password (last resort):** `docker exec -u root <container> bash -c 'echo "admin:NEW" | chpasswd'` does NOT work for Jenkins — the password lives in `/var/jenkins_home/users/admin_*/config.xml` as a hashed `passwordHash`. To reset it, edit that file with a known hash (or run Jenkins with `-Djenkins.install.runSetupWizard=true` and reuse `initialAdminPassword`). Avoid this unless the password is genuinely lost.

### Minting a long-lived API token

After the initial setup wizard is complete:

1. Open **Manage Jenkins → Users → admin → Configure**
2. Scroll to **API Token → Add new Token**
3. Give it a name (e.g. `bootstrap-token`) and click **Generate**
4. **Copy the token now** — it is shown only once and cannot be retrieved later

#### Path B — mint via REST

If you prefer not to use the GUI, you can mint the token via the Jenkins REST API. The cookie jar is required — the CSRF crumb is only honored within the same session that issued it.

```bash
# Pre-req: the admin password (initialAdminPassword or the configured one).
JENKINS_URL=http://10.0.0.8:8080
JENKINS_USER=admin
read -rsp "Jenkins admin password: " JENKINS_PASS
echo
# Read at the prompt — keeps the password out of shell history.

cookies=$(mktemp)
crumb=$(curl -sS -c "$cookies" -u "$JENKINS_USER:$JENKINS_PASS" \
  "$JENKINS_URL/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

curl -sS -b "$cookies" -u "$JENKINS_USER:$JENKINS_PASS" -H "$crumb" \
  -X POST --data 'newTokenName=bootstrap-token' \
  "$JENKINS_URL/user/$JENKINS_USER/descriptorByName/jenkins.security.ApiTokenProperty/generateNewToken"
# → JSON containing tokenValue. Copy that value into JENKINS_TOKEN in ci/jenkins/.env.
rm -f "$cookies"
```

### Storing credentials locally

```bash
cp ci/jenkins/.env.example ci/jenkins/.env
# Then open ci/jenkins/.env and fill in JENKINS_TOKEN with the value from above.
```

`ci/jenkins/.env` is listed in `.gitignore` — it will never be committed.

**URL note:** LAN/remote addresses (e.g., `http://10.0.0.8:8080`) are valid substitutions for `http://localhost:8080`. Strip any trailing slash — `${JENKINS_URL}/createItem` would otherwise produce `//createItem`, which some reverse proxies reject.

**Job name:** Use `oxide-sloc` for the SCM-driven job created from `ci/jenkins/job-config.xml`. Use `oxide-sloc-manual` only if you also intend to maintain a hand-edited copy of the pipeline in the same Jenkins instance and need to disambiguate.

---

## Step 0 — Pre-flight check

Run this before the `createItem` call. It verifies reachability, authentication, plugin presence, and that no conflicting job exists:

```bash
set -a; source ci/jenkins/.env; set +a && bash ci/jenkins/preflight.sh
```

All lines must print `[ok]`. Fix any `[fail]` before continuing.

---

## Installing plugins

Required plugins are listed in `ci/jenkins/plugins.txt`. The full set includes:

| Plugin | Purpose |
|--------|---------|
| `workflow-aggregator` | Declarative Pipeline syntax |
| `pipeline-utility-steps` | `readJSON` in `post { success }` |
| `git` | SCM checkout |
| `ws-cleanup` | `cleanWs()` in `post { cleanup }` |
| `credentials-binding` | SMTP / webhook credential bindings |
| `htmlpublisher` | "SLOC Report" sidebar link |
| `plot` | Build-over-build trend charts |
| `pipeline-stage-view` | Stage view in the job UI |
| `timestamper` | Timestamps on console output |
| `copyartifact` | Copy artifacts to/from downstream jobs |
| `bitbucket` | Bitbucket Branch Source (optional) |
| `bitbucket-build-status-notifier` | Bitbucket build status (optional) |
| `ansicolor` | ANSI color in Rust compiler output |

### Path 1 — Docker (online)

```bash
docker exec -u root <container> jenkins-plugin-cli \
  --plugins $(grep -Ev '^#|^$' ci/jenkins/plugins.txt | awk '{print $1}' | tr '\n' ' ')
```

### Path 2 — Docker (air-gapped)

```bash
# On a networked machine — download each .hpi:
while IFS= read -r line; do
  [[ "$line" =~ ^#|^$ ]] && continue
  id=$(echo "$line" | awk '{print $1}')
  curl -L -o "${id}.hpi" "https://updates.jenkins.io/latest/${id}.hpi"
done < ci/jenkins/plugins.txt

# Transfer .hpi files to the air-gapped host, then install:
docker exec -u root <container> \
  cp /host/path/<plugin>.hpi /var/jenkins_home/plugins/<plugin>.hpi
# Restart Jenkins to load the plugins.
```

Alternatively: **Manage Jenkins → Plugins → Advanced → Deploy Plugin → Upload .hpi file**

### Path 3 — Native / systemd install (Jenkins CLI jar)

Use this when Jenkins runs directly on the host (not in Docker):

```bash
set -a; source ci/jenkins/.env; set +a
curl -sS -o jenkins-cli.jar "${JENKINS_URL}/jnlpJars/jenkins-cli.jar"
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" \
    install-plugin $(grep -Ev '^#|^$' ci/jenkins/plugins.txt | awk '{print $1}')
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" safe-restart
```

Wait ~30 seconds for Jenkins to come back up, then re-run `preflight.sh` (check c verifies all plugins are active).

### Plugin verification

`ci/jenkins/preflight.sh` check (c) queries the plugin manager and prints `[ok]` / `[fail]` per plugin. Run it after any plugin install before proceeding. You can also verify manually:

```bash
set -a; source ci/jenkins/.env; set +a
curl -su "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/pluginManager/api/json?depth=1" \
  | python3 -c "
import json, sys
data = json.load(sys.stdin)
required = [l.split()[0] for l in open('ci/jenkins/plugins.txt') if l.strip() and not l.startswith('#')]
installed = {p['shortName']: p.get('active') and p.get('enabled') for p in data['plugins']}
for r in required:
    status = 'ok' if installed.get(r) else ('disabled' if r in installed else 'MISSING')
    print(f'  {status:8s}  {r}')
missing = [r for r in required if not installed.get(r)]
if missing: sys.exit(1)
"
```

Exit code 0 = all plugins active. Non-zero = at least one missing or disabled.

---

## Option A — Job DSL

See `ci/jenkins/seed-job.groovy`.

---

## Option B — CLI bootstrap

### Step 1 — Create the job

```bash
set -a; source ci/jenkins/.env; set +a

# Render the job XML with your REPO_URL substituted
bash ci/jenkins/render-job-config.sh   # writes /tmp/job-config.xml

CRUMB=$(curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    -H "${CRUMB}" -H "Content-Type: application/xml" \
    --data-binary @/tmp/job-config.xml \
    "${JENKINS_URL}/createItem?name=${JOB_NAME}"
```

A 200 response with an empty body means the job was created successfully. A 400 with `job already exists` means the job name is taken — choose a different `JOB_NAME`.

### Step 2 — Trigger the first (seed) build

```bash
curl -sS -X POST -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/job/${JOB_NAME}/build"
```

The first build runs with no parameters — Jenkins uses it to discover the `parameters {}` block in the Jenkinsfile. From build #2 onward, **Build with Parameters** in the left-hand sidebar shows the full configurable form.

---

## Setting the artifact-viewer CSP

The HTML report requires the Jenkins artifact viewer to allow inline styles. The recommended approach is to drop `ci/jenkins/init.groovy.d/relax-csp.groovy` into `$JENKINS_HOME/init.groovy.d/` before starting Jenkins:

```bash
cp ci/jenkins/init.groovy.d/relax-csp.groovy $JENKINS_HOME/init.groovy.d/
# Then restart Jenkins.
```

This sets the CSP property at startup without requiring in-process script approval. For external origins (GitHub Pages, S3), control the `Content-Security-Policy` response header directly on that service instead.

---

## Bitbucket integration

### Required plugins

- **Bitbucket Branch Source** (`bitbucket`) — enables Bitbucket multibranch projects
- **Bitbucket Build Status Notifier** (`bitbucket-build-status-notifier`) — posts commit statuses to Bitbucket

### Webhook URL

Configure a webhook in Bitbucket pointing to:
```
<JENKINS_URL>/bitbucket-hook/
```

For Bitbucket Server (Data Center), navigate to **Repository Settings → Webhooks → Add webhook** and set the URL above. For Bitbucket Cloud, use **Repository Settings → Webhooks**.

### Environment variables

Set these in `ci/jenkins/.env` (see `.env.example`):

```bash
export BITBUCKET_URL=https://bitbucket.example.com   # Server/Data Center URL
export BITBUCKET_PROJECT=OXIDE                        # Project key
export BITBUCKET_REPO=oxide-sloc                      # Repository slug
```

### Using Bitbucket as the SCM source

In `ci/jenkins/seed-job.groovy`, uncomment and fill in the `bitbucketServer { }` block:

```groovy
bitbucketServer {
    serverUrl(System.getenv('BITBUCKET_URL') ?: 'https://bitbucket.example.com')
    credentialsId('bitbucket-credentials')
    projectKey(System.getenv('BITBUCKET_PROJECT') ?: 'OXIDE')
    repositoryName(System.getenv('BITBUCKET_REPO') ?: 'oxide-sloc')
}
```

Add a credential with ID `bitbucket-credentials` (username + password or SSH key) via **Manage Jenkins → Credentials**.
