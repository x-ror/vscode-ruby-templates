# Ручне тестування: фічі та демо-проєкти

Чекліст усього, що вміє розширення, з файлом для кожної перевірки, діями та очікуваним результатом. У кожної перевірки є ID (`N-07`), щоб посилатися на неї в баг-репорті.

Опис поведінки: [README](../README.md). Якщо чекліст і README розходяться, розбіжність варто повідомити окремим багом.

## Що тут лежить

| Папка                                                                    | Для чого                                                                                                                                               |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `rails_app/`                                                             | Мінімальний Rails-застосунок без Rails: `Gemfile.lock` з `actionview 8.0`, контролери, хелпери, компоненти, Stimulus, локалі, `db/schema.rb`, маршрути |
| `rails_app/app/views/users`, `shared`, `admin`, `layouts`, `user_mailer` | В'ю для навігації, автодоповнення й hover (ERB, Haml, Slim, Jbuilder)                                                                                  |
| `rails_app/engines/blog`, `rails_app/app/views/blog`                     | Engine в репозиторії (`path:`-гем у `Gemfile`) з `blog/_post` і `blog/_sidebar`; застосунок перекриває `_sidebar` (`users/engines.html.erb`)           |
| `rails_app/lib/reports`                                                  | ERB з рядками `%` для мови `erb-percent` (її вмикає `files.associations` у `.vscode/settings.json`)                                                    |
| `shop_app/`, `demo.code-workspace`                                       | Другий Rails-застосунок для вікна з двома застосунками: свої партіали, переклади, маршрути, `render "x"` без папки                                     |
| `rails_app/app/views/playground/broken`                                  | По одній проблемі на файл для перевірки діагностики                                                                                                    |
| `rails_app/app/views/playground/format`                                  | Погано відформатовані файли та файли, які форматер має лишити без змін                                                                                 |
| `rails_app/app/views/playground/editing`                                 | Команди редагування: toggle, wrap, extract partial, коментарі в `.js.erb`, `.css.erb`… і в багаторядкових тегах                                        |
| `rails_app/app/views/playground/editorconfig`                            | Однаковий файл під чотирма секціями `.editorconfig` (глоби)                                                                                            |
| `rails_app/app/views/playground/liquid`                                  | Liquid: тема магазину й Jekyll-include (`_includes/header.html`)                                                                                       |
| `rails_app/app/views/playground/highlight`                               | Зразки підсвітки (копії `test/sample.*`)                                                                                                               |
| `rails_app/app/views/playground/preview`                                 | Шаблони без Rails-хелперів для Preview / Show Compiled Ruby Source                                                                                     |
| `rails_app/app/views/playground/stimulus`                                | Атрибути Stimulus-контролерів елемента (`search_controller.js`) в ERB, Haml і Slim                                                                     |
| `rails_app/bin/rails`                                                    | Заглушка: відповідає лише на запит маршрутів, щоб `*_path` працювали без встановленого Rails                                                           |
| `no_rails/`                                                              | Шаблони без `Gemfile.lock`: Rails-перевірки тут вимкнені                                                                                               |

## 0. Підготовка

1. Запуск зі збіркою з цього репозиторію (потрібні Rust, Bun і `code` у `PATH`):

   ```sh
   demo/run.sh                      # rails_app
   demo/run.sh no_rails             # інший демо-проєкт
   demo/run.sh demo.code-workspace  # rails_app і shop_app в одному вікні
   ```

   Скрипт збирає сервер (`bun run build:server`) і клієнт (`bun run build`) і відкриває вікно _Extension Development Host_. Розширення з цієї папки має пріоритет над встановленим з Marketplace (у тебе зараз стоїть 0.3.2).

   Інший варіант, ближчий до того, що отримають користувачі: `bun run package` і `code --install-extension ruby-templates-*.vsix`.

   У вікні з двома папками діють лише налаштування з `demo.code-workspace` (там продубльовано `files.associations` для `erb-percent` і Liquid); решта з `rails_app/.vscode/settings.json` ні.

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
- [ ] **H-07** `sample.html.erb`, рядки 26–34: тіло `<<~SQL` у багаторядковому `<% … %>` підсвічене як SQL (`SELECT`, `FROM`), `#{id}` як Ruby, тіло `<<~HTML.html_safe` як HTML, а `%>` на окремому рядку закриває тег. `<p>after heredoc</p>` після нього звичайний HTML. Рядок 36 `% complete …` лишається текстом, не Ruby.
- [ ] **H-08** `sample.percent.erb` (мова `erb-percent` із `files.associations`, у статус-барі «Text (ERB, % lines)»): рядки 1 і 3 (`% items.each …`, `% end`) підсвічені як Ruby, `%% not code` не Ruby, ` % indented` (з пробілом) і `100 % complete` лишаються текстом.
- [ ] **H-09** `sample.html.liquid` (у статус-барі «Liquid»): `{{ }}` з фільтрами, рядками, числами й діапазонами `(1..5)`, теги `{% %}`, `{% comment %}` і `{% # %}`, тіло `{% raw %}` не підсвічене як Liquid, багаторядковий `{% liquid %}`, Liquid усередині `<script>`, `<style>` і значень атрибутів. `Ctrl+/` на рядку дає `{% comment %}`, а `{{` і `{%` автоматично закриваються.
- [ ] **H-10** Семантична підсвітка (колір залежить від теми; тип токена видно в _Inspect Editor Tokens and Scopes_). `users/_card.html.erb`: `compact` (рядки 2 і 4) та `user` (3 і 5) мають тип `parameter`, а сам коментар `locals: (user:, compact: false)` у рядку 1 лишається без токенів. Змінні блоку мають тип `variable`: `user` у `users/index.html.erb` (10–12) і `c` (15–17), `f` і `tag` у `users/edit.html.slim` (3–9, 15–16), `role` у `users/show.html.haml` (13–14). Strict-локали в Slim токенів не мають. `editor.semanticHighlighting.enabled: false` повертає колір самої граматики.

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
- [ ] **E-13** `Ctrl+/` коментує за мовою хоста (назву мови видно в статус-барі, наприклад «CSS (ERB)»): `comments.js.erb` рядок 3 і `comments.scss.erb` рядок 3 дають `// …`, `comments.css.erb` рядок 2 дає `/* … */`, `comments.rb.erb` рядок 2 і `comments.yml.erb` рядок 3 дають `# …`, `comments.xml.erb` рядок 2 і `comments.md.erb` рядок 3 дають `<!-- … -->`. У `highlight/sample.json.erb` і `sample.text.erb` лишається `<%# … %>`. Повторне `Ctrl+/` знімає коментар.
- [ ] **E-14** `multiline.html.erb`: виділи рядки 3–4 (всередині `<% … %>`) і натисни `Ctrl+/`, вони стають `    # count = …` і `    # label = …` (спільний відступ збережено, один `# `). Те саме для рядка 7 (`users_path,` усередині `<%= link_to … %>`). Повторне `Ctrl+/` знімає `# `. Рядки з `<%` і `%>` (2, 5, 6, 8) і поза тегами (1, 9) коментуються як раніше, а не через `# `.

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
- [ ] **D-23** `heredoc_end_tag.html.erb`: `%>` усередині heredoc (рядок 3) закриває тег, як в Erubi, тому Ruby-помилки «unterminated string meets end of file» і «unexpected end-of-input …» (рядок 5). Правильні heredoc у багаторядкових тегах (`format/heredoc.html.erb`, `highlight/sample.html.erb` 26–34) діагностик не дають.
- [ ] **D-24** `html_recovery.html.erb`: зайвий `</span>` (рядок 2) не зупиняє перевірку тегів, далі йдуть `<img>` без `alt` (3), «End tag `</h2>` doesn't close the open `<h3>`» (4) і «`<div>` is never closed» (5). Разом чотири діагностики.
- [ ] **D-25** `html_wrapper.html.erb`: обгортка з гілками (`<% if link %><a><% else %><span><% end %>` … `</a>` / `</span>`, рядки 1–3) і з запереченою умовою (`if !admin` … `unless admin`, рядки 4–6) не дають попереджень, а перевірка йде далі: лише `<img>` без `alt` (7) і «`<div>` is never closed» (8).
- [ ] **D-26** `stray_end.html.erb`: «unexpected 'end', ignoring it» підкреслює сам `end` у рядку 2 (колонка 4), а не останній рядок файлу.
- [ ] **D-27** `lib/reports/broken.erb` (мова `erb-percent`, див. H-08 і S-06): помилка Ruby стоїть на рядку 2, у `% total = (users.size + * 2)`: «unexpected integer; expected an expression after the operator». `lib/reports/summary.erb` діагностик не має, хоч `% if` … `<% else -%>` … `% end` це один блок. Якщо перемкнути мову на звичайний _ERB_ (_Change Language Mode_), у `summary.erb` з'явиться «unexpected 'else'» (рядок 8).

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
- [ ] **D-28** `render_folder.html.erb`: у `rails_app` попереджень немає: `Gemfile.lock` має `PATH`-гем `blog` з `app/views`, тож Rails може знайти папку `uesrs` в одному з додаткових шляхів, і сервер мовчить (так само вимкнено перевірки голих імен). Щоб побачити «Missing partial uesrs/_card» у рядку 1, тимчасово прибери з `Gemfile.lock` блок `PATH` і рядок `blog!` (рядок 2, `users/card`, без попередження в обох випадках); поверни файл після перевірки. `devise/shared/links` у `new.html.erb:6` не підкреслюється ніколи (гем із `Gemfile.lock` може мати власну папку в'ю).
- [ ] **D-29** `format_missing.html.erb`: `formats: [:xml]` (рядок 1) і `handlers: [:slim]` (рядок 2) дають «Missing partial users/_badge», бо `_badge` є лише як `.html.erb`, `.html+phone.erb`, `.json.jbuilder`, `.html.haml`. Quick fix створює `users/_badge.xml.erb` і `users/_badge.html.slim`. Ті самі виклики з наявним форматом (`users/formats.html.erb`) без попереджень.
- [ ] **D-30** `engine_render.html.erb` (партіали engine `blog`): рядок 1 «missing local: :post for app/views/blog/_post.html.erb» (сигнатура береться з engine). Рядок 2 (`blog/summary`, якого немає) не підкреслюється: `blog` це гем із `Gemfile.lock`, а гем може мати власні в'ю. Без блоку `PATH` у `Gemfile.lock` (див. D-28) з'являється й «Missing partial blog/_summary». Власні в'ю engine (`engines/blog/app/views/…`) render-перевірок не мають.
- [ ] **D-31** `$B check demo/shop_app` (див. розділ 7) дає єдине попередження: `pages/index.html.erb:2`, `render "banner"` → «Missing partial pages/_banner, application/_banner». `products/index.html.erb:3`, `render "badge"` без попередження: `ProductsController` робить `include Themeable`, а `lib/themeable.rb` викликає `prepend_view_path`, тож Rails шукає ще й у `themes/` (там `products/_badge.html.erb`). `PagesController` нічого не підключає, тому там перевірка працює.
- [ ] **D-32** Без файлу: `Ctrl+N`, мова _ERB_, встав `<img src="x">`, `<div>` і `<%= render "users/card" %>` окремими рядками. З'являються «`<img>` without `alt`» і «`<div>` is never closed» (HTML-перевірки діють і в безіменному документі). Hover і F12 на `"users/card"` нічого не дають, а Output без помилок.

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
- [ ] **N-19** `admin/reports/index.html.erb`: набери `<%= @` → `@reports`, `@admin_section` (з `Admin::BaseController`), `@current_user`. Батьківський контролер записано без модуля (`class ReportsController < BaseController` у `module Admin`), і він усе одно знаходиться.
- [ ] **N-41** Папки без контролера: `shared/_nav.html.haml`, рядок 4, hover на `@current_user` показує `@current_user = User.first` із посиланням на `application_controller.rb:8`, F12 переходить туди. Те саме в `layouts/application.html.erb`, рядок 8 (`<body class=…>`). Новий рядок `= @` у Haml-файлі (`<%= @` в ERB, наприклад у `shared/_footer.html.erb` чи layout) пропонує лише `@current_user` (колбек без `only:`), без `@users`, `@user` чи `@admin_section`. `layouts/mailer.html.erb`, рядок 5: hover і F12 на `@company` ведуть на `application_mailer.rb:9`, а `<%= @` пропонує `@company`.
- [ ] **N-42** Колонки моделі. `users/index.html.erb`, рядок 12: hover на `name` в `user.name` дає `` `users.name` — string ``, F12 відкриває `db/schema.rb:3`; запис `@users = User.all` береться з `users_controller.rb:5`, а колонку знаходять у схемі. `users/show.html.haml`, рядок 5: `@user.email` дає `users.email`, F12 на `db/schema.rb:5`; рядок 6 `@user.admin` дає `` `users.admin` — boolean, default: false ``. `users/edit.html.slim`, рядок 12: `@user.email` так само. `admin/reports/index.html.erb`, рядок 5: `report.title` дає `` `reports.title` — string, null: false ``, F12 на `db/schema.rb:11`. Нічого не показується на зв'язках `@user.tags` (`edit.html.slim:15`) і `@user.roles` (`show.html.haml:13`): це `has_many` у `user.rb`, а не колонки. Так само на `user.name` у `users/_card.html.erb:3` (strict local, запис невідомий).

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
- [ ] **N-43** Hover на хелпер застосунку. `users/index.html.erb`, рядок 21: `format_price` показує `def format_price(amount, currency: "USD")`, посилання на `users_helper.rb:3` і коментар над методом. Рядок 30: `user_label` показує сигнатуру одним рядком (у файлі параметри на двох). Рядок 4 і `shared/_nav.html.haml:5`: `current_user_name` показує `def current_user_name` і «`helper_method` in `ApplicationController`» з посиланням на `application_controller.rb:11`. Hover на `link_to` лишається документацією Rails API (N-31).
- [ ] **N-28** `users/edit.html.slim` у формі: набери `= f.` → методи form builder. Набери `= f.text_field :` → `name`, `nickname`, `email`, `admin`, `created_at` з `db/schema.rb`.
- [ ] **N-29** Набери `<%= turbo_stream.` → `append`, `replace`, `update`…
- [ ] **N-30** `data-controller="` → `hello`, `clipboard`. `data-action="click->hello#` → `greet`, `reset`. `data-hello-target="` → `name`, `output`.
- [ ] **N-31** Hover на `link_to` → сигнатура й посилання на API docs. Набери `<%= link_` → хелпери з аргументами-сніпетами. У виклику, де аргументи вже є (`<%= link_ user.name, edit_user_path(user) %>`), вибір `link_to` вставляє лише ім'я, без сніпета перед аргументами.
- [ ] **N-32** У `@users.each do |user|` (рядки 10–13) набери `<%= us` → `user`.
- [ ] **N-39** `playground/stimulus/attributes.html.erb`: у тезі `<form data-controller="search"` (рядки 2–4) після `data-search-url-value="…"` набери ` data-search-` → `data-search-target`, `data-search-url-value`, `data-search-debounce-wait-value`, `data-search-loading-class`, `data-search-result-list-outlet` (з `static targets/values/classes/outlets` у `search_controller.js`; слова з коментаря всередині `static values` не потрапляють). У `<div data-controller="search hello" <%= … %>` (рядок 9) перед `class=` набери `data-` → атрибути обох контролерів, ERB-тег їм не заважає.
      Haml (`attributes.html.haml`): у `%form(` на другому рядку атрибутів набери `data-search-` → ті самі імена. У `%form{ data: { … } }` (рядок 6) після `search_url_value: "/users/search"` набери `, search_` → `search_target`, `search_debounce_wait_value`, `search_loading_class`… Slim (`attributes.html.slim`): у `form(` на другому рядку набери `data-search-`.
      Slim також доповнює Turbo-атрибути: у `input ` набери `data-turbo-` → `data-turbo-method`, `data-turbo-confirm`, `data-turbo-frame`…
- [ ] **N-40** `users/index.html.erb`: F12 на `user` у `user.name` (рядок 12) → `|user|` на рядку 10. У `users/_card.html.erb` F12 на `user` у `user.name` (рядок 3) як і раніше веде на `render`, що передають `user:` (strict local, не блокова змінна).

**Інші корені в'ю, формати, посилання**

- [ ] **N-44** Engine в репозиторії (`engines/blog`, підключений у `Gemfile` і `Gemfile.lock` як `path:`-гем). `users/engines.html.erb`: F12 на `"blog/post"` (рядок 3) відкриває `engines/blog/app/views/blog/_post.html.erb`, hover показує цей шлях від кореня застосунку й `locals: (post:)`. F12 на `"blog/sidebar"` (рядок 5) відкриває `app/views/blog/_sidebar.html.erb`: партіал застосунку перекриває engine. Набери `<%= render "blog/` → `blog/post`, `blog/sidebar` (по одному разу), а після `render "blog/post", ` → `post:`. В `engines/blog/app/views/blog/posts/index.html.erb` F12 на `"users/card"` (рядок 5) веде в `app/views/users/_card.html.erb`. Партіали гема-engine (`devise/shared/links`) знаходяться так само, але лише якщо гем встановлено (`vendor/bundle` чи `GEM_HOME`); у демо його немає.
- [ ] **N-45** `render` з `formats:`, `variants:`, `handlers:`. `users/formats.html.erb`, F12 на `"users/badge"`: рядок 2 (без опцій) дає список `_badge.html.erb`, `.html.haml`, `.html+phone.erb`. Рядок 3 (`formats: [:json]`) дає `_badge.json.jbuilder`. Рядок 4 (`variants: [:phone]`) ставить `_badge.html+phone.erb` першим. Рядок 5 (`handlers: [:haml]`) дає лише `_badge.html.haml`. Рядок 6 (опції перед іменем, `"badge"` без папки) дає `_badge.json.jbuilder`. У рядку 7 (`render "note", formats: [:json]`) хеш це locals, як у Rails, тому `formats` тут лише локальна змінна, а F12 веде в `_note.html.erb` (партіал без strict locals; з `_badge`, де є `locals: (user:)`, такий виклик був би помилкою). Рядок 8, `"/users/card"` → `_card.html.erb`. Hover у рядку 3 показує `app/views/users/_badge.json.jbuilder`.
- [ ] **N-46** Перенесення партіала в `app/views`. `shared/_notice.html.erb` викликають два різні layout-и (вони не рендеряться разом). У Explorer перетягни файл у `app/views`: `render "shared/notice"` у `layouts/application.html.erb` (рядок 10) і `layouts/admin.html.slim` (рядок 7) стають `render "/notice"`. F12 на `"/notice"` відкриває `app/views/_notice.html.erb`. Після перевірки поверни файл (`git checkout -- demo && git clean -fd demo`).
- [ ] **N-47** Підказки посилань (`Ctrl`/`Cmd` + наведення) показують шлях від кореня застосунку, а не повний шлях на диску: `"users/card"` (`users/index.html.erb:6`) → `app/views/users/_card.html.erb`, `data-controller="hello"` (там само, рядок 24) → `app/javascript/controllers/hello_controller.js`, `stylesheet_link_tag "application"` (`layouts/application.html.erb:6`) → `app/assets/stylesheets/application.css`, `layout "admin"` (`admin/base_controller.rb:3`) → `app/views/layouts/admin.html.slim`.
- [ ] **N-48** Два застосунки (`demo/run.sh demo.code-workspace`). У `shop_app/app/views/products/index.html.erb` усе береться зі `shop_app`: F12 на `"users/card"` (рядок 4) і `"shared/footer"` (5) відкриває файли `shop_app/app/views/…`, а не `rails_app/…`; після `render "users/card", ` пропонується `customer:` (не `user:`); hover на `t(".title")` (2) показує лише `en: Products`; hover на `about_path` (6) дає `/about` → `pages#index`, F12 веде в `shop_app/config/routes.rb:5`. У `rails_app` лишається своє (N-01…N-25): `new_user_path` там є, а `about_path` ні.
- [ ] **N-49** Liquid не дає відповідей Rails. `playground/liquid/product.html.liquid`: F12, hover і `Ctrl`+клік на `"card"` (рядок 5) та `"users/card"` (6) нічого не роблять, посилань немає, у Problems порожньо, _Format Document_ нічого не змінює. `playground/liquid/_includes/header.html` це Liquid лише завдяки `files.associations` (S-07): звичайні `.html` розширення не забирає.

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
- [ ] **F-11** `.js.erb`, `.text.erb`, `.json.erb`, `*.jbuilder`, `.liquid` не форматуються ніколи (`highlight/sample.js.erb`, `highlight/sample.html.liquid`).
- [ ] **F-12** `heredoc.html.erb`: рядки з початком тегів `<%= <<~HTML.html_safe` (2) і `<% summary = <<~TEXT` (6) та `<p>` (10) отримують один рівень відступу. Тіла heredoc, їхні кінцівки (`HTML`, `TEXT`) і рядки `%>` (3–5, 7–9) лишаються байт у байт.
- [ ] **F-13** Глоби `.editorconfig`. У VS Code ширину задає розмір табуляції редактора (F-05), тому дивись через командний рядок (розділ 7): у `playground/editorconfig` `plain.html.erb` дає відступ 4 (секція `[app/views/playground/editorconfig/**/*.html.erb]`: `/**/` збігається і з однією `/`), `page2.html.erb` 3 (`[page{1..3}.html.erb]`: без `/` секція діє в будь-якій папці), `page02.html.erb` знову 4 (діапазон не збігається з нулем попереду), `{brace.html.erb` 6 (секція `[{brace.html.erb]` з незакритою дужкою читається буквально).

## 6. Команди, яким потрібен Ruby (`playground/preview`)

Потрібні геми: `gem install erubi slim haml` (прев'ю), `html2haml haml_to_erb html2slim haml2slim` (конвертери). Без гема команда має показати зрозумілу помилку, а не тихо нічого не робити.

- [ ] **P-01** `Ctrl+Alt+P` у `plain.html.erb`, `.haml`, `.slim` → HTML у боковій панелі, оновлюється при збереженні.
- [ ] **P-02** _Show Compiled Ruby Source_ на тих самих файлах.
- [ ] **P-03** _Convert ERB/HTML → Haml_, _Convert ERB/Haml → Slim_, _Convert Haml/Slim → ERB_ на `plain.*`.
- [ ] **P-04** Прев'ю в'ю з Rails-хелперами (`users/index.html.erb`) без Rails показує помилку `link_to`/`render`. Це очікувано. `rubyTemplates.preview.useRails` потребує справжнього застосунку (`rails new`), бо заглушка `bin/rails` відповідає лише на маршрути.
- [ ] **P-05** `lib/reports/summary.erb` (мова `erb-percent`): `Ctrl+Alt+P` показує текст `Users: 2`, `  - Ann`, `  - Bob`, `Done.`, `% not a Ruby line` (stdlib ERB з `trim_mode: "%-"`: рядки `%` це Ruby, `%%` це літерал). _Show Compiled Ruby Source_ показує рядки `%` як код.

## 7. Командний рядок

Бінарник після `bun run build:server`: `server/target/release/ruby-templates`. Запускати з кореня репозиторію.

```sh
B=server/target/release/ruby-templates
$B check demo/rails_app/app/views                               # exit 1, список з playground/broken, playground/format/unsafe.* і users/new
$B check demo/rails_app/app/views/users                         # exit 1, рівно 3 попередження з new.html.erb
$B check demo/rails_app/app/views/playground/preview            # exit 0, нічого
$B check demo/rails_app/app/views/playground/liquid             # exit 0, Liquid CLI не перевіряє
$B check demo/shop_app                                          # exit 1, одне попередження: pages/index.html.erb:2 (D-31)
$B check demo/rails_app/lib                                     # exit 1: summary.erb:8, CLI читає його як Erubi-ERB, а не erb-percent
$B check demo/rails_app/config/routes.rb                        # exit 2, unknown template type
$B format --check demo/rails_app/app/views/playground/format    # exit 1: чотири файли (три messy й heredoc), два "left unchanged"
$B format demo/rails_app/app/views/playground/format/messy.html.slim   # друкує відформатований файл
$B format --verbose --check demo/rails_app/app/views/playground/format # плюс налаштування й прочитаний .editorconfig
$B format --verbose --check demo/rails_app/app/views/playground/editorconfig 2>&1 | grep -o 'editorconfig/[^:]*: Settings { indent_size: [0-9]*'   # відступи 4, 3, 4, 6 (F-13)
printf '<div>\n<p>x</p>\n</div>\n' | $B format --stdin-filepath show.html.erb
$B format --write demo/rails_app/app/views/playground/format    # переписує файли (потім git checkout)
```

- [ ] **C-01** Вивід і коди виходу збігаються з коментарями вище.
- [ ] **C-02** Повторний `format --check` після `--write` дає exit 0 (форматування стабільне).
- [ ] **C-03** `$B check demo/rails_app/app/views | head -1` не панікує (stderr порожній).

## 8. Налаштування (`rails_app/.vscode/settings.json`)

- [ ] **S-01** `"rubyTemplates.erb.disabledDiagnostics": ["img-missing-alt"]`: попередження про `alt` зникають, решта лишається. Те саме з кодом з Problems для Haml, Slim і `rubyTemplates.jbuilder.disabledDiagnostics`.
- [ ] **S-02** `"rubyTemplates.slim.diagnostics.enabled": false`: жодної діагностики в `.slim` і `.skim`.
- [ ] **S-03** `"rubyTemplates.erb.autoCloseTags": false`: E-02 більше не додає закривальний тег.
- [ ] **S-04** `rubyTemplates.slim.extraEngines` містить `mjml`: у Slim `mjml:` не є помилкою «unknown engine» і є в автодоповненні.
- [ ] **S-05** `"rubyTemplates.server.enabled": false`: підсвітка й сніпети працюють, діагностики, навігації та форматування немає.
- [ ] **S-06** `"files.associations": { "**/lib/reports/*.erb": "erb-percent" }`: `lib/reports/summary.erb` має мову _Text (ERB, % lines)_. Без запису файл відкривається як звичайний ERB: рядки `%` це текст, а діагностика дає «unexpected 'else'» (D-27).
- [ ] **S-07** `"files.associations": { "**/playground/liquid/_includes/*.html": "liquid" }`: `_includes/header.html` має мову _Liquid_. Без запису це звичайний HTML.

## Відомі проблеми

Відкритих issue, через які перевірки падають, зараз немає. Раніше відкриті виправлено, і для кожного є перевірка: [#69](https://github.com/x-ror/vscode-ruby-templates/issues/69) (зайвий `<% end %>`) D-26, [#70](https://github.com/x-ror/vscode-ruby-templates/issues/70) (батьківський контролер без модуля) N-19, [#71](https://github.com/x-ror/vscode-ruby-templates/issues/71) (F12 на змінній блоку) N-40, [#72](https://github.com/x-ror/vscode-ruby-templates/issues/72) (колонки моделі) N-42, [#73](https://github.com/x-ror/vscode-ruby-templates/issues/73) (хелпер у заповненому виклику) N-31, [#74](https://github.com/x-ror/vscode-ruby-templates/issues/74) (ivar-и в `shared/` і `layouts/`) N-41, [#75](https://github.com/x-ror/vscode-ruby-templates/issues/75) (hover на хелперах застосунку) N-43. Якщо одна з них знову падає, це регресія, її варто репортити.

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
