// oxide-sloc — Jenkins Job DSL seed job
//
// Creates (or updates) the oxide-sloc pipeline job via the Job DSL plugin.
//
// Usage:
//   1. Install the "Job DSL" plugin.
//   2. Create a Freestyle project named "oxide-sloc-seed" (or similar).
//   3. Under Build Steps → Process Job DSLs, point it at this file
//      (using "Look on filesystem" or paste the content).
//   4. Run the seed job once — it creates the oxide-sloc pipeline job.
//   5. Run the generated job once with no parameters to seed the form;
//      the full "Build with Parameters" form appears from build #2 onward.
//
// Alternatively, paste this script directly into:
//   Manage Jenkins → Script Console (for one-shot execution without a seed job).

pipelineJob('oxide-sloc') {
    description('oxide-sloc SLOC analysis pipeline. ' +
                'Scans source repositories and produces HTML, JSON, and PDF reports ' +
                'with build-over-build trend data.')

    logRotator {
        numToKeep(25)
        artifactNumToKeep(10)
    }

    definition {
        cpsScm {
            scm {
                git {
                    remote {
                        url('https://github.com/oxide-sloc/oxide-sloc.git')
                    }
                    branch('*/main')
                    extensions {
                        cloneOptions {
                            shallow(false)
                            timeout(10)
                        }
                    }
                }
            }
            scriptPath('Jenkinsfile')
            lightweight(true)
        }
    }
}
