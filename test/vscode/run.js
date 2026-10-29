// Runs test/vscode/suite.js inside a real VS Code (downloaded once into .vscode-test/) on a fake Rails app.
// Linux needs a display: `xvfb-run -a npm run test:vscode`.
import fs from "fs";
import os from "os";
import path from "path";
import { runTests } from "@vscode/test-electron";

const root = path.join(import.meta.dirname, "..", "..");
const app = fs.mkdtempSync(path.join(os.tmpdir(), "rt-vscode-"));
const write = (rel, text) => {
  const file = path.join(app, rel);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
};
write(
  "app/views/users/index.html.erb",
  '<div>\n<%= render "users/card" %>\n<% if x %>\n',
);
write("app/views/users/_card.html.erb", "<p>card</p>\n");
write("app/views/user_mailer/welcome.html.erb", "<p>Hi</p>\n");
write("app/views/user_mailer/welcome.text.erb", "Hi\n");
write("app/views/orders/show.html.erb", "<p>Order</p>\n");
// Stands in for `bin/rails runner routes.rb`: the client only needs its marker and JSON.
write(
  "bin/rails",
  'puts "booted", "__RUBY_TEMPLATES_ROUTES__", \'[{"name":"users","verb":"GET","path":"/users(.:format)","action":"users#index","source":"config/routes.rb:2"}]\'\n',
);
write(
  "config/routes.rb",
  "Rails.application.routes.draw do\n  resources :users\nend\n",
);
write("app/views/users/routes.html.erb", '<%= link_to "All", users_path %>\n');
write(
  "app/controllers/orders_controller.rb",
  "class OrdersController < ApplicationController\n  def index\n  end\n\n  def show\n  end\nend\n",
);
write("app/components/badge_component.rb", "class BadgeComponent; end\n");
write("app/components/card_component.rb", "class CardComponent; end\n");
write(
  "spec/components/previews/card_component_preview.rb",
  "class CardComponentPreview; end\n",
);
write(
  "app/components/badge_component.html.erb",
  "<span><%= content %></span>\n",
);
write("app/views/users/toggle.html.erb", "<% x %>\n");
write(
  "app/views/users/unformatted.html.erb",
  "<div>\n<% if x %>\n<p>a</p>\n<% end %>\n</div>\n",
);
write("app/views/users/comment.html.erb", "<div>\n<!-- a\n  b -->\n</div>\n");
write("app/views/users/wrap.html.erb", "hello world\n");
write("app/views/users/wrap_empty.html.erb", "\n");
write(
  "app/views/users/toggle.html.slim",
  "- foo\n= bar\n== baz\n/ qux\njavascript:\n  - not a marker\n",
);
write("app/views/users/wrap.html.slim", "p\n  hello world\n");
write("app/views/users/toggle.html.haml", "- foo\n= bar\n-# baz\n%p\n");
write("app/views/users/reindent.html.haml", "%div\n    %p\n        %b x\n");
write("app/views/users/format.html.haml", "%ul\n   %li\n      %b x\n");
write("app/views/users/format.html.slim", "div\n    p= x\n    p y\n");
write(
  "app/views/users/extract.html.erb",
  "<div>\n  <p>a</p>\n  <p>b</p>\n</div>\n",
);
write("app/views/users/extract.html.haml", "%div\n  %p a\n  %p b\n");
write("app/views/users/extract.html.slim", "div\n  p a\n  p b\n");
try {
  await runTests({
    version: process.env.VSCODE_VERSION ?? "stable",
    extensionDevelopmentPath: root,
    extensionTestsPath: path.join(import.meta.dirname, "suite.js"),
    launchArgs: [app, "--disable-extensions", "--disable-workspace-trust"],
    extensionTestsEnv: { RT_APP: app },
  });
} catch (e) {
  console.error(e.message ?? e);
  process.exitCode = 1;
} finally {
  fs.rmSync(app, { recursive: true, force: true });
}
