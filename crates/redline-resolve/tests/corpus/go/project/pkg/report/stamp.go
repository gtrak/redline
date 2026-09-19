package report

// Version is the report's own build stamp, declared at package level.
// In report.go the dot-imported auxversion.Version shadows it (file
// scope wins); the overlap is exactly what makes the bare Version use
// ambiguous to the import-context handoff.
const Version = "report-1.0.0"
