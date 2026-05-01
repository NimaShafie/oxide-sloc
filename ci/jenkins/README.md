# Jenkins job bootstrap

## Operator workflow (overview)

1. `cp ci/jenkins/.env.example ci/jenkins/.env` — fill in `JENKINS_TOKEN`
2. `source ci/jenkins/.env && bash ci/jenkins/preflight.sh` — all checks must pass
3. Run the `createItem` curl (Step 1 below)
4. Run the seed-build curl (Step 2 below)

That's it. Nothing else is needed.

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

### Minting a long-lived API token

After the initial setup wizard is complete:

1. Open **Manage Jenkins → Users → admin → Configure**
2. Scroll to **API Token → Add new Token**
3. Give it a name (e.g. `bootstrap-token`) and click **Generate**
4. **Copy the token now** — it is shown only once and cannot be retrieved later

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
source ci/jenkins/.env && bash ci/jenkins/preflight.sh
```

All lines must print `[ok]`. Fix any `[fail]` before continuing.

---

## Installing plugins

All 7 required plugins are listed in `ci/jenkins/plugins.txt`.

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
source ci/jenkins/.env
curl -sS -o jenkins-cli.jar "${JENKINS_URL}/jnlpJars/jenkins-cli.jar"
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" \
    install-plugin $(grep -Ev '^#|^$' ci/jenkins/plugins.txt | awk '{print $1}')
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" safe-restart
```

Wait ~30 seconds for Jenkins to come back up, then re-run `preflight.sh` (check c verifies all plugins are active).

### Plugin verification

`ci/jenkins/preflight.sh` check (c) queries the plugin manager and prints `[ok]` / `[fail]` per plugin. Run it after any plugin install before proceeding. You can also verify manually:

```bash
source ci/jenkins/.env
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
source ci/jenkins/.env

CRUMB=$(curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    -H "${CRUMB}" -H "Content-Type: application/xml" \
    --data-binary @ci/jenkins/job-config.xml \
    "${JENKINS_URL}/createItem?name=${JOB_NAME}"
```

A 200 response with an empty body means the job was created successfully. A 400 with `job already exists` means the job name is taken — choose a different `JOB_NAME`.

### Step 2 — Trigger the first (seed) build

```bash
curl -sS -X POST -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/job/${JOB_NAME}/build"
```

The first build runs with no parameters — Jenkins uses it to discover the `parameters {}` block in the Jenkinsfile. From build #2 onward, **Build with Parameters** in the left-hand sidebar shows the full configurable form.
