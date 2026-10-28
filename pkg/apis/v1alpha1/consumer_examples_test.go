// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package v1alpha1

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"gopkg.in/yaml.v3"
)

func TestProductConsumerExamplesMatchTaskContract(t *testing.T) {
	t.Parallel()

	for _, product := range []string{"sites", "coach"} {
		product := product
		t.Run(product, func(t *testing.T) {
			t.Parallel()

			path := filepath.Join("..", "..", "..", "examples", "consumers", product+"-task.yaml")
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			var task Task
			if err := yaml.Unmarshal(data, &task); err != nil {
				t.Fatalf("strictly decode %s: %v", path, err)
			}
			if err := ValidateTask(&task); err != nil {
				t.Fatalf("validate %s: %v", path, err)
			}
			if task.GetApiVersion() != APIVersion || task.GetKind() != KindTask {
				t.Fatalf("unexpected contract identity %q %q", task.GetApiVersion(), task.GetKind())
			}
			if !strings.Contains(task.GetSpec().GetImage(), "@sha256:") {
				t.Fatalf("consumer image is not immutable: %q", task.GetSpec().GetImage())
			}
			if task.GetSpec().GetDebug() {
				t.Fatal("product consumer examples must default debug off")
			}
			wantEnv := map[string]bool{
				"PRODUCT_SESSION_ID":            false,
				"PRODUCT_SESSION_GENERATION":    false,
				"PRODUCT_SESSION_FENCING_TOKEN": false,
				"MODEL_GATEWAY_BASE_URL":        false,
			}
			for _, env := range task.GetSpec().GetEnv() {
				if _, ok := wantEnv[env.GetName()]; ok {
					wantEnv[env.GetName()] = true
				}
			}
			for name, found := range wantEnv {
				if !found {
					t.Errorf("missing required opaque integration input %s", name)
				}
			}
		})
	}
}
