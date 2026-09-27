package substrate

import "testing"

func TestBuildActorTemplateHardensWorkload(t *testing.T) {
	tmpl := BuildActorTemplate("space", "template", "image", nil, []string{"run"}, "bucket")
	sc := tmpl.GetContainers()[0].GetSecurityContext()
	if sc.GetRunAsUser() != 65532 || sc.GetRunAsGroup() != 65532 {
		t.Fatalf("workload identity = %d:%d, want 65532:65532", sc.GetRunAsUser(), sc.GetRunAsGroup())
	}
	if !sc.GetReadOnlyRootFilesystem() || !sc.GetNoNewPrivileges() {
		t.Fatalf("workload hardening = readonly:%v no-new-privileges:%v, want true/true", sc.GetReadOnlyRootFilesystem(), sc.GetNoNewPrivileges())
	}
	if got := sc.GetCapabilities().GetDrop(); len(got) != 1 || got[0] != "ALL" {
		t.Fatalf("dropped capabilities = %v, want [ALL]", got)
	}
}
