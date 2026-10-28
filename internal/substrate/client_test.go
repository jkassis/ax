package substrate

import (
	"testing"

	"github.com/google/ax/pkg/apis/v1alpha1"
)

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

func TestEgressRulesPreservePublicDestinationPortsAndDenyEmptyPolicy(t *testing.T) {
	rules, err := egressRules(&v1alpha1.EgressAllowlist{Hosts: []*v1alpha1.HostRule{
		{Host: "*", Port: 80},
		{Host: "*", Port: 443},
		{Host: "api.example.com", Port: 8443},
	}})
	if err != nil {
		t.Fatalf("egressRules: %v", err)
	}
	if len(rules) != 3 {
		t.Fatalf("rules = %d, want 3", len(rules))
	}
	for i, port := range []int32{80, 443} {
		if rules[i].GetPublic() == nil || len(rules[i].GetPorts()) != 1 || rules[i].GetPorts()[0] != port {
			t.Errorf("public rule %d = %+v, want port %d", i, rules[i], port)
		}
	}
	if got := rules[2]; got.GetHostnames().GetPatterns()[0] != "api.example.com" || got.GetPorts()[0] != 8443 {
		t.Errorf("hostname rule = %+v", got)
	}
	empty, err := egressRules(&v1alpha1.EgressAllowlist{})
	if err != nil || len(empty) != 0 {
		t.Fatalf("empty allowlist = (%v, %v), want an explicit deny-all policy", empty, err)
	}
}

func TestEgressRulesRejectInvalidHostOrPort(t *testing.T) {
	for _, rule := range []*v1alpha1.HostRule{
		nil,
		{Host: "", Port: 443},
		{Host: "*", Port: 0},
		{Host: "*", Port: 65536},
	} {
		if _, err := egressRules(&v1alpha1.EgressAllowlist{Hosts: []*v1alpha1.HostRule{rule}}); err == nil {
			t.Errorf("egressRules(%+v) succeeded, want error", rule)
		}
	}
}
