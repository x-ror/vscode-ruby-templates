# Ручне тестування: фічі та демо-проєкти

Чекліст усього, що вміє розширення, з файлом для кожної перевірки, діями та очікуваним результатом. У кожної перевірки є ID (`N-07`), щоб посилатися на неї в баг-репорті.

Повний опис поведінки: [docs/FEATURES.md](../docs/FEATURES.md). Якщо чекліст і FEATURES.md розходяться, правдою вважається FEATURES.md, а розбіжність варто повідомити окремим багом.

## Що тут лежить

| Папка                                                                    | Для чого                                                                                                                                               |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rails_app/`                                                             | Мінімальний Rails-застосунок без Rails: `Gemfile.lock` з `actionview 8.0`, контролери, хелпери, компоненти, Stimulus, локалі, `db/schema.rb`, маршрути |
| `rails_app/app/views/users`, `shared`, `admin`, `layouts`, `user_mailer` | В'ю для навігації, автодоповнення й hover (ERB, Haml, Slim, Jbuilder)                                                                                  |
| `rails_app/app/views/playground/broken`                                  | По одній проблемі на файл для перевірки діагностики                                                                                                    |
| `rails_app/app/views/playground/format`                                  | Погано відформатовані файли та файли, які форматер має лишити без змін                                                                                 |
| `rails_app/app/views/playground/editing`                                 | Команди редагування: toggle, wrap, extract partial                                                                                                     |
| `rails_app/app/views/playground/highlight`                               | Зразки підсвітки (копії `test/sample.*`)                                                                                                               |
| `rails_app/app/views/playground/preview`                                 | Шаблони без Rails-хелперів для Preview / Show Compiled Ruby Source                                                                                     |
| `rails_app/bin/rails`                                                    | Заглушка: відповідає лише на запит маршрутів, щоб `*_path` працювали без встановленого Rails                                                           |
| `no_rails/`                                                              | Шаблони без `Gemfile.lock`: Rails-перевірки тут вимкнені                                                                                               |

## 0. Підготовка

1. Запуск зі збіркою з цього репозиторію (потрібні Rust, Bun і `code` у `PATH`):

   ```sh
   demo/run.sh            # rails_app
   demo/run.sh no_rails   # інший демо-проєкт
   ```

   Скрипт збирає сервер (`bun run build:server`) і клієнт (`bun run build`) і відкриває вікно _Extension Development Host_. Розширення з цієї папки має пріоритет над встановленим з Marketplace (у тебе зараз стоїть 0.3.2).

   Інший варіант, ближчий до того, що отримають користувачі: `bun run package` і `code --install-extension ruby-templates-*.vsix`.

2. На запит _Do you trust the authors?_ натисни **Trust**: маршрути читаються лише в довіреній папці.
3. Логи: _View → Output_, канал **Ruby Templates**. Сервер перезапускає _Ruby Templates: Restart Language Server_.
4. Тести змінюють демо-файли (rename, форматування, quick fix). Повернути все як було:

   ```sh
   git checkout -- demo && git clean -fd demo
   ```

## 1. Підсвітка (`playground/highlight`)

Відкрий кожен `sample.*` і пошукай місця, де колір явно неправильний: Ruby як текст, HTML як Ruby, «поїхала» підсвітка до кінця файлу. Scope під курсором показує _Developer: Inspect Editor Tokens and Scopes_.

- [ ] **H-01** `sample.html.erb`: `<% %>`, `<%= %>`, `<%# %>`, `<%%` поверх HTML.
- [ ] **H-02** `sample.html.haml`: `%tag.class#id`, `{}` / `()` атрибути, фільтри `:javascript`, `:css`, `:ruby`, `#{}`.
- [ ] **H-03** `sample.html.slim`: багаторядкові атрибути, `|` текст, `javascript:` з `#{}`, коментарі `/`, `/!`.
- [ ] **H-04** `sample.js.erb`, `sample.json.erb`, `sample.text.erb`: мова-хост (JS, JSON, текст) плюс ERB.
- [ ] **H-05** `sample.skim`: як Slim, але CoffeeScript замість Ruby.
- [ ] **H-06** `rails_app/app/javascript/controllers/*.js` і `.rb`-файли лишаються за своїми розширеннями (Ruby LSP), а `*.jbuilder` має мову `ruby`.

## 2. Редагування (`playground/editing`)

- [ ] **E-01** `erb.html.erb`: набери `<%`, має з'явитися `<% %>`.
- [ ] **E-02** `erb.html.erb`: набери `<section>`, після `>` має додатися `</section>`. Для `<br>` і `<img>` нічого не додається.
- [ ] **E-03** `erb.html.erb`: перейменуй `div` у рядку 4 на `article`, закривальний `</div>` має змінитися теж (linked editing увімкнено в `.vscode/settings.json`).
- [ ] **E-04** `Ctrl+/` у рядку з HTML дає `<!-- -->`, у рядку `<% %>` дає ERB-коментар. Те саме в Haml (`-#`) і Slim (`/`).
- [ ] **E-05** `Ctrl+Alt+E` на `<% user.name %>` (рядок 2): `<%` → `<%=` → `<%#` → `<%`.
- [ ] **E-06** Виділи `user.email` (рядок 3) і натисни `Ctrl+Alt+W`, має вийти `<%= user.email %>`.
- [ ] **E-07** Виділи блок `<div class="box">…</div>`, _Ruby Templates: Extract Selection to Partial_, ім'я `box`. З'являється `playground/editing/_box.html.erb`, на місці виділення стоїть `render`.
- [ ] **E-08** `haml.html.haml`: `Ctrl+Alt+E` у рядку 2 перемикає `-` → `=` → `-#`. Extract Selection to Partial на двох `%p` створює `_name.html.haml` без зайвого відступу.
- [ ] **E-09** `slim.html.slim`: `Ctrl+Alt+E` у рядку 2 перемикає `-` → `=` → `==` → `/`. `Ctrl+Alt+W` на рядку 3 цілком дає `= user.email`, на виділеному `user.name` у рядку 4 дає `#{user.name}`.
- [ ] **E-10** `slim.html.slim`: виділи два `p` у `section`, _Extract Selection to Slim Partial_, ім'я `shared/lines`. Файл з'являється в `app/views/shared/_lines.html.slim`, у `section` лишається `= render "shared/lines"`.
- [ ] **E-11** Сніпети: у новому рядку `.html.erb` набери `link_to`, `form_with`, `ff`, `turbo_frame`, `ts-append`, `locals`, `stimulus` і натисни Tab. Те саме в Haml і Slim.
- [ ] **E-12** Emmet: `ul>li*3` + Tab у `.html.erb` і в `.html.slim` (у Slim має вийти Slim-синтаксис).

## 3. Діагностика (`playground/broken`, Problems panel)

Кожна діагностика має код (у Problems він у дужках). Каскад повідомлень Prism на одну Ruby-помилку (2–5 рядків) очікуваний.

**ERB**

- [ ] **D-01** `html_attrs.html.erb`: `<img>` без `alt` (рядок 1), `image_tag` без `alt:` (2), info про `<img src="<%= … %>">` (3), duplicate id `main` (5), info про `<%==`, `raw`, `.html_safe` (6–8).
- [ ] **D-02** `html_attrs.html.erb` рядок 9: на `<%=@user.name%>` є code action _Add spaces inside this ERB tag_ (лампочка / `Ctrl+.`).
- [ ] **D-03** `html_mismatch.html.erb`: «End tag `</div>` doesn't close the open `<span>`».
- [ ] **D-04** `html_unexpected_end.html.erb`: «Unexpected end tag `</span>`».
- [ ] **D-05** `html_unclosed.html.erb`: «`<section>` is never closed».
- [ ] **D-06** `html_void_end.html.erb`: «End tag `</br>` for a void element».
- [ ] **D-07** `html_p_closed.html.erb`: `<div>` закрив `<p>`, тому `</p>` у рядку 3 зайвий.
- [ ] **D-08** `ruby_syntax.html.erb`, `ruby_two_statements.html.erb`: Ruby-помилки в тегах, з позицією в шаблоні.
- [ ] **D-09** `ruby_percent_in_string.html.erb`: `%>` усередині рядка закриває тег, тому помилка.
- [ ] **D-10** `ruby_if_in_output.html.erb`: `<%= if %>` дає ``expected `then` or `;` or '\n'``.
- [ ] **D-11** `unclosed_erb_tag.html.erb`: попередження про `<%` без `%>`.
- [ ] **D-12** `blocks.html.erb`: блок без `<% end %>`, зайвий `end`, `if` у `<%= %>`, незакритий `<%`.
- [ ] **D-13** `strict_locals_syntax.html.erb`: «`a` set as non-keyword argument … Locals can only be set as keyword arguments.»

**Haml / Slim**

- [ ] **D-14** `bad.html.haml`: «Illegal nesting: content can't be both given on the same line as %span and nested within it.» на рядку 5. Haml, як і сам Haml, показує лише першу помилку: виправ її і дивись, що з'явиться далі (`- end`, невідомий фільтр, `- else` без `if`, `%br/` з текстом, вкладення під `=`…).
- [ ] **D-15** `bad.html.slim`: «Unexpected indentation», «Expected closing delimiter )», «Explicit end statements are forbidden». Виправляй по одній і дивись наступні (таби, невідомий engine, поганий dedent).
- [ ] **D-16** `ruby.html.haml`, `ruby.html.slim`: Ruby-помилки в `=`, атрибутах, `#{}`, неправильний `- rescue`. Підкреслення стоїть у місці помилки, а не в колонці 1.

**Rails**

- [ ] **D-17** `users/new.html.erb`: рядок 3 `missing local: :user`, рядок 4 `unknown local: :colour`, рядок 5 `Missing partial users/_missing_partial`. Рядки 2 і 6 (`devise/…`, гем є в `Gemfile.lock`) без попереджень.
- [ ] **D-18** Quick fix (`Ctrl+.`) на рядку 3: _Pass `user` to the render_ додає `user: nil`.
- [ ] **D-19** Quick fix на рядку 4: _Remove `colour` from the render_ та _Declare `colour` in the locals of `_card.html.erb`_.
- [ ] **D-20** Quick fix на рядку 5: _Create partial `users/_missing_partial.html.erb`_ створює файл, попередження зникає.
- [ ] **D-21** `no_rails/views/index.html.erb` (`demo/run.sh no_rails`): `render "does/not/exist"` без попередження (немає `Gemfile.lock`), `<img>` без `alt` з попередженням.
- [ ] **D-22** Діагностика оновлюється під час набору та після зміни `Gemfile.lock` (наприклад, видали рядок `actionview`: попередження про render мають зникнути).

## 4. Rails-навігація (`rails_app`)

Номери рядків для `users/index.html.erb`. «Набери» означає новий рядок, щоб отримати автодоповнення (`Ctrl+Space`, якщо список не відкрився сам).

**Партіали**

- [ ] **N-01** F12 на `"users/card"` (рядок 6) відкриває `users/_card.html.erb`. Hover показує файл.
- [ ] **N-02** F12 на `"card"` (рядок 11, відносний шлях) і на `partial: "shared/footer"` (рядок 7).
- [ ] **N-03** F12 на `render @users` (рядок 8) відкриває `users/_user.html.erb`.
- [ ] **N-04** Набери `<%= render "` → список партіалів (`users/card`, `shared/footer`, `shared/nav`, …).
- [ ] **N-05** Набери `<%= render "users/card", ` → пропонує `user:` і `compact:` (з дефолтом `false`), яких ще немає у виклику.
- [ ] **N-06** Shift+F12 на `"users/card"` або будь-де в `_card.html.erb` → усі `render`, що на нього вказують (`index`, `show.html.haml`, `new`, …).
- [ ] **N-07** У `_card.html.erb` F12 на `user` → список `render`, які передають `user:`. Hover на `compact` у `locals:` пише «default `false`» і кількість викликів.
- [ ] **N-08** F2 на `compact` у `_card.html.erb` → `dense`: змінюються `locals:`, читання в партіалі та ключ `compact: true` в `index.html.erb:11`. F2 на ім'я, яке вже є (`user`), або на `end` відмовляє.
- [ ] **N-09** У Explorer перейменуй `shared/_footer.html.erb` → `_bottom.html.erb`: `render "shared/footer"` у `layouts/application.html.erb` і `partial: "shared/footer"` в `index.html.erb` стають `shared/bottom`. Перейменування папки `shared` → `common` теж оновлює виклики.
- [ ] **N-10** Формат з ініціалізатора: в `users/index.mobile.erb` F12 на `"users/card"` відкриває `_card.mobile.erb` (`Mime::Type.register … :mobile` у `config/initializers/mime_types.rb`).
- [ ] **N-11** `Ctrl+T`, набери `uscard` → `users/card (partial)`.
- [ ] **N-12** Jbuilder: `users/index.json.jbuilder` F12 на `"users/user"` → `_user.json.jbuilder`, те саме для `json.partial!` у `show.json.jbuilder`.

**ViewComponent**

- [ ] **N-13** F12 на `CardComponent` (рядок 15) → `card_component.rb` / `.html.erb`. F12 на `UI::ButtonComponent` (рядок 19, акронім) → `ui/button_component.rb`.
- [ ] **N-14** Набери `<%= render Car` → `CardComponent.new(…)`.
- [ ] **N-15** Усередині блоку (рядок 16) набери `<% c.` → `with_header`, `with_item`, `with_items`.
- [ ] **N-16** У `card_component.html.erb` набери `<%= hea` → `header`, `header?`.

**Instance variables**

- [ ] **N-17** `index.html.erb`: набери `<%= @` → `@users`, `@total`, `@current_user` (з `ApplicationController`), але не `@user` (`before_action :set_user, only: %i[show edit]`).
- [ ] **N-18** `users/show.html.haml`: hover на `@user` показує `@user = User.find(params[:id])` з посиланням на рядок, F12 переходить туди.
- [ ] **N-19** `admin/reports/index.html.erb`: набери `<%= @` → `@reports`, `@admin_section` (з `Admin::BaseController`), `@current_user`.

**Маршрути** (через заглушку `bin/rails`, потрібен `ruby` у `PATH` і довірена папка)

- [ ] **N-20** Набери `<%= link_to "x", us` → `users_path`, `user_path`, `users_url`… з методом і шляхом.
- [ ] **N-21** Hover на `edit_user_path` (рядок 12) → `/users/:id/edit`, **GET** → `users#edit`. Hover на `users_path` у `shared/_nav.html.haml` показує GET і POST.
- [ ] **N-22** F12 на `new_user_path` (рядок 22) → `config/routes.rb:4`.
- [ ] **N-23** Зміни `config/routes.rb` і збережи: в Output має з'явитися новий запуск `bin/rails runner`. Якщо маршрути не працюють, причина теж буде там (`[routes] …`).
- [ ] **N-24** `rubyTemplates.routes.enabled: false` вимикає маршрути. У недовіреному вікні (_Workspaces: Manage Workspace Trust_) `bin/rails` не запускається.

**i18n, хелпери, форми, Stimulus**

- [ ] **N-25** Hover на `t(".title")` (рядок 2) → значення для `en` і `uk`. F12 → `config/locales/*.yml`. Shift+F12 → виклики й ключ у кожній локалі (зокрема `t("users.index.title")` в `index.mobile.erb`).
- [ ] **N-26** Набери `<%= t(".` → ключі `users.index.*`.
- [ ] **N-27** F12 на `format_price` (рядок 21) → `app/helpers/users_helper.rb`. F12 на `current_user_name` (рядок 4) → `helper_method` в `ApplicationController`.
- [ ] **N-28** `users/edit.html.slim` у формі: набери `= f.` → методи form builder. Набери `= f.text_field :` → `name`, `nickname`, `email`, `admin`, `created_at` з `db/schema.rb`.
- [ ] **N-29** Набери `<%= turbo_stream.` → `append`, `replace`, `update`…
- [ ] **N-30** `data-controller="` → `hello`, `clipboard`. `data-action="click->hello#` → `greet`, `reset`. `data-hello-target="` → `name`, `output`.
- [ ] **N-31** Hover на `link_to` → сигнатура й посилання на API docs. Набери `<%= link_` → хелпери з аргументами-сніпетами.
- [ ] **N-32** У `@users.each do |user|` (рядки 10–13) набери `<%= us` → `user`.

**Контролери, суміжні файли, outline**

- [ ] **N-33** `users_controller.rb`: F12 на `def show` → `show.html.haml` і `show.json.jbuilder` (список). F12 на `render :new` → `new.html.erb`.
- [ ] **N-34** `admin/base_controller.rb`: F12 на `layout "admin"` → `layouts/admin.html.slim`.
- [ ] **N-35** _Switch to Related File_: `users/index.html.erb` ↔ `index.json.jbuilder` / `index.mobile.erb` / `UsersController#index`. `user_mailer/welcome.html.erb` ↔ `welcome.text.erb` / `UserMailer#welcome`. `card_component.rb` ↔ `.html.erb` ↔ `test/components/previews/card_component_preview.rb`.
- [ ] **N-36** Outline і breadcrumbs у `users/show.html.haml`, `users/edit.html.slim`, `layouts/application.html.erb`.
- [ ] **N-37** Hover у Slim на маркері рядка (`-`, `=`, `/`, `|`, `javascript:`) пояснює його. Hover у Haml на `:javascript`, `=`, `-`.
- [ ] **N-38** Haml-автодоповнення: `%` → теги, `:` → фільтри, `%a{` → атрибути (`href`, `data:`). Slim: `inp` на початку рядка → тег, `input(ty` → атрибут, `- link_` → хелпер (у тексті `| link_` хелперів немає).

## 5. Форматування (`playground/format`)

_Format Document_ (`Shift+Alt+F`). Після кожного файлу відкат: `Ctrl+Z` або `git checkout`.

- [ ] **F-01** `messy.html.erb`: один рівень відступу на HTML-елемент і ERB-блок, `<% else %>` / `<% end %>` на рівні `if`, хвостові пробіли прибрані. Тіла `<pre>` і `<script>` та друге й наступні рядки багаторядкового `<%= f.text_field … %>` не змінюються. Порожні рядки всередині ERB лишаються (ERB-форматер їх не стискає).
- [ ] **F-02** `messy.html.haml`: відступ 2, тіло `:javascript` рухається разом із заголовком і зберігає свій відносний відступ, порожні рядки в кінці зникають.
- [ ] **F-03** `messy.html.slim`: відступ 2, `|`-блок і тіло `javascript:` рухаються з рядком. Хвостові пробіли в `|`-тексті лишаються, бо вони частина виводу.
- [ ] **F-04** `unsafe.html.erb`, `unsafe.html.slim`: файл не змінюється, в Output причина («left unchanged …»).
- [ ] **F-05** Ширина відступу: у статус-барі _Spaces: 2_ → _Indent Using Spaces: 4_, потім Format Document дає 4. Без редактора ширина береться з `rails_app/.editorconfig`.
- [ ] **F-06** _Format Selection_ в ERB і Haml форматує лише виділене.
- [ ] **F-07** Format on save: `"[erb]": { "editor.formatOnSave": true }` (і `[haml]`, `[slim]`) у `.vscode/settings.json`.
- [ ] **F-08** `"editor.codeActionsOnSave": { "source.erbSpacing": "explicit" }`: при збереженні `broken/html_attrs.html.erb` тег `<%=@user.name%>` стає `<%= @user.name %>`.
- [ ] **F-09** `rubyTemplates.erb.format.alignComments: true`: рядки всередині багаторядкового `<%# … %>` рухаються разом із його першим рядком.
- [ ] **F-10** `rubyTemplates.<erb|haml|slim>.format.enabled: false`: Format Document більше нічого не робить (або пропонує інший форматер).
- [ ] **F-11** `.js.erb`, `.text.erb`, `.json.erb`, `*.jbuilder` не форматуються ніколи (`highlight/sample.js.erb`).

## 6. Команди, яким потрібен Ruby (`playground/preview`)

Потрібні геми: `gem install erubi slim haml` (прев'ю), `html2haml haml_to_erb html2slim haml2slim` (конвертери). Без гема команда має показати зрозумілу помилку, а не тихо нічого не робити.

- [ ] **P-01** `Ctrl+Alt+P` у `plain.html.erb`, `.haml`, `.slim` → HTML у боковій панелі, оновлюється при збереженні.
- [ ] **P-02** _Show Compiled Ruby Source_ на тих самих файлах.
- [ ] **P-03** _Convert ERB/HTML → Haml_, _Convert ERB/Haml → Slim_, _Convert Haml/Slim → ERB_ на `plain.*`.
- [ ] **P-04** Прев'ю в'ю з Rails-хелперами (`users/index.html.erb`) без Rails показує помилку `link_to`/`render`. Це очікувано. `rubyTemplates.preview.useRails` потребує справжнього застосунку (`rails new`), бо заглушка `bin/rails` відповідає лише на маршрути.

## 7. Командний рядок

Бінарник після `bun run build:server`: `server/target/release/ruby-templates`. Запускати з кореня репозиторію.

```sh
B=server/target/release/ruby-templates
$B check demo/rails_app/app/views                               # exit 1, список з playground/broken і users/new
$B check demo/rails_app/app/views/users                         # exit 1, рівно 3 попередження з new.html.erb
$B check demo/rails_app/app/views/playground/preview            # exit 0, нічого
$B check demo/rails_app/config/routes.rb                        # exit 2, unknown template type
$B format --check demo/rails_app/app/views/playground/format    # exit 1: три messy-файли, два "left unchanged"
$B format demo/rails_app/app/views/playground/format/messy.html.slim   # друкує відформатований файл
$B format --verbose --check demo/rails_app/app/views/playground/format # плюс налаштування й прочитаний .editorconfig
printf '<div>\n<p>x</p>\n</div>\n' | $B format --stdin-filepath show.html.erb
$B format --write demo/rails_app/app/views/playground/format    # переписує файли (потім git checkout)
```

- [ ] **C-01** Вивід і коди виходу збігаються з коментарями вище.
- [ ] **C-02** Повторний `format --check` після `--write` дає exit 0 (форматування стабільне).
- [ ] **C-03** `$B check demo/rails_app/app/views | head -1` не панікує.

## 8. Налаштування (`rails_app/.vscode/settings.json`)

- [ ] **S-01** `"rubyTemplates.erb.disabledDiagnostics": ["img-missing-alt"]`: попередження про `alt` зникають, решта лишається. Те саме з кодом з Problems для Haml, Slim і `rubyTemplates.jbuilder.disabledDiagnostics`.
- [ ] **S-02** `"rubyTemplates.slim.diagnostics.enabled": false`: жодної діагностики в `.slim` і `.skim`.
- [ ] **S-03** `"rubyTemplates.erb.autoCloseTags": false`: E-02 більше не додає закривальний тег.
- [ ] **S-04** `rubyTemplates.slim.extraEngines` містить `mjml`: у Slim `mjml:` не є помилкою «unknown engine» і є в автодоповненні.
- [ ] **S-05** `"rubyTemplates.server.enabled": false`: підсвітка й сніпети працюють, діагностики, навігації та форматування немає.

## Відомі проблеми

Уже відкриті issue. Перевірки, які через них падають, не треба репортити вдруге.

| Issue                                                                                                                            | Перевірка | Що                                                                                                          |
| -------------------------------------------------------------------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------- |
| [#69](https://github.com/x-ror/vscode-ruby-templates/issues/69)                                                                  | D-12      | Зайвий `<% end %>` показано на останньому рядку файлу                                                       |
| [#70](https://github.com/x-ror/vscode-ruby-templates/issues/70)                                                                  | N-19      | ivar-и з батьківського контролера, записаного без модуля (`< BaseController` у `module Admin`)              |
| [#73](https://github.com/x-ror/vscode-ruby-templates/issues/73)                                                                  | N-31      | Автодоповнення хелпера в уже заповненому виклику вставляє сніпет перед аргументами                          |
| [#71](https://github.com/x-ror/vscode-ruby-templates/issues/71), [#72](https://github.com/x-ror/vscode-ruby-templates/issues/72) |           | Немає F12 на змінній блоку і на колонці моделі (`report.title`); те, що показується, приходить від Ruby LSP |
| [#74](https://github.com/x-ror/vscode-ruby-templates/issues/74), [#75](https://github.com/x-ror/vscode-ruby-templates/issues/75) |           | Немає ivar-ів у `shared/` і `layouts/`, немає hover на хелперах застосунку (поки під питанням)              |

## Баг-репорт

[Issues](https://github.com/x-ror/vscode-ruby-templates/issues). Шаблон:

````markdown
**Check**: N-21 (або «поза чеклістом»)
**Version**: commit `git rev-parse --short HEAD` / розширення з Marketplace X.Y.Z; VS Code `code --version`; OS

**Steps**

1. Open `demo/rails_app/app/views/users/index.html.erb`
2. Hover `edit_user_path` on line 12

**Expected**: `/users/:id/edit`, GET → users#edit
**Actual**: no hover

**Minimal template** (if it isn't a demo file):

```erb
…
```

**Logs**: Output → Ruby Templates (the lines around the problem)
**CLI** (for diagnostics and formatting): `ruby-templates check <file>` / `ruby-templates format --verbose <file>` output
````

Перед тим як відправити:

- Перевір, чи відтворюється в CLI. Якщо так, це баг сервера, і проблемний файл разом із виводом CLI замінить будь-які скриншоти.
- Скороти шаблон до кількох рядків, на яких баг ще відтворюється.
- Для підсвітки додай скриншот і scope з _Inspect Editor Tokens and Scopes_.
- Для невідповідності Slim/Haml/Rails порівняй із самим движком: `slimrb -c file.slim`, `haml compile file.haml`.
