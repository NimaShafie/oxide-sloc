# Jenkins job bootstrap

## Option A — Job DSL

See `ci/jenkins/seed-job.groovy`.

## Option B — CLI (curl)

Requires curl, a Jenkins admin account or API token, and write access to Jenkins.

```bash
JENKINS_URL=http://localhost:8080
JENKINS_USER=admin
JENKINS_PASS=<password-or-api-token>
JOB_NAME=oxide-sloc           # change to oxide-sloc-manual etc. if desired
JAR=$(mktemp)

# 1. Obtain a CSRF crumb (must share session with the POST below)
CRUMB=$(curl -su "${JENKINS_USER}:${JENKINS_PASS}" -c "$JAR" -b "$JAR" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

# 2. Create the job
curl -su "${JENKINS_USER}:${JENKINS_PASS}" -c "$JAR" -b "$JAR" \
    -H "${CRUMB}" -H "Content-Type: application/xml" \
    --data-binary @ci/jenkins/job-config.xml \
    "${JENKINS_URL}/createItem?name=${JOB_NAME}"
```

> **Note:** If you authenticate with a Jenkins API token (not a password), the crumb is not session-bound and the cookie jar is unnecessary — use the token in the Authorization header and skip `-c`/`-b`.

After the job is created, trigger one build with no parameters so Jenkins discovers the `parameters{}` block in the Jenkinsfile — the full "Build with Parameters" form appears from build #2 onward.
