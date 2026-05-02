// Drop into $JENKINS_HOME/init.groovy.d/ before starting Jenkins.
// Sets the artifact-viewer CSP so HTML reports render with inline styles.
System.setProperty(
    'hudson.model.DirectoryBrowserSupport.CSP',
    "default-src 'self'; style-src 'self' 'unsafe-inline'; " +
    "img-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; " +
    "font-src 'self' data:;"
)
