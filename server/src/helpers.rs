//! Static Rails view helper completions.

/// `module` is where the method is documented (checked against the actionview / turbo-rails sources).
pub struct Helper { pub name: &'static str, pub detail: &'static str, pub snippet: &'static str, pub module: Option<&'static str> }

macro_rules! h {
    ($n:literal, $d:literal, $s:literal) => { Helper { name: $n, detail: $d, snippet: $s, module: None } };
    ($n:literal, $d:literal, $s:literal, $m:literal) => { Helper { name: $n, detail: $d, snippet: $s, module: Some($m) } };
}

impl Helper {
    /// api.rubyonrails.org for Rails, rubydoc.info for turbo-rails.
    pub fn doc_url(&self) -> Option<String> {
        let module = self.module?;
        if module.starts_with("Turbo::") {
            return Some(format!("https://www.rubydoc.info/gems/turbo-rails/{}#{}-instance_method", module.replace("::", "/"), self.name));
        }
        // RDoc anchors percent-encode the name with `-` for `%`: `content_for?` → `content_for-3F`.
        let anchor = self.name.replace('?', "-3F");
        Some(format!("https://api.rubyonrails.org/classes/{}.html#method-i-{anchor}", module.replace("::", "/")))
    }
}

pub const HELPERS: &[Helper] = &[
    h!("link_to", "link_to(name, options = nil, html_options = nil)", "link_to \"${1:text}\", ${2:path}", "ActionView::Helpers::UrlHelper"),
    h!("button_to", "button_to(name, options, html_options)", "button_to \"${1:text}\", ${2:path}, method: :${3:post}", "ActionView::Helpers::UrlHelper"),
    h!("mail_to", "mail_to(email, name = nil)", "mail_to \"${1:email}\"", "ActionView::Helpers::UrlHelper"),
    h!("image_tag", "image_tag(source, options = {})", "image_tag \"${1:file.png}\", alt: \"${2}\"", "ActionView::Helpers::AssetTagHelper"),
    h!("video_tag", "video_tag(sources, options)", "video_tag \"${1:file.mp4}\"", "ActionView::Helpers::AssetTagHelper"),
    h!("render", "render(options = {}, locals = {}, &block)", "render \"${1:partial}\"", "ActionView::Helpers::RenderingHelper"),
    h!("form_with", "form_with(model: nil, scope: nil, url: nil, format: nil, **options, &block)", "form_with model: ${1:@record} do |f|", "ActionView::Helpers::FormHelper"),
    h!("form_for", "form_for(record, options = {}, &block)", "form_for ${1:@record} do |f|", "ActionView::Helpers::FormHelper"),
    h!("fields_for", "fields_for(record_name, record_object = nil, options = {}, &block)", "fields_for :${1:association} do |ff|", "ActionView::Helpers::FormHelper"),
    h!("content_for", "content_for(name, content = nil, options = {}, &block)", "content_for :${1:name} do", "ActionView::Helpers::CaptureHelper"),
    h!("content_for?", "content_for?(name)", "content_for?(:${1:name})", "ActionView::Helpers::CaptureHelper"),
    h!("yield", "yield(name = nil)", "yield${1: :${2:name}}"),
    h!("provide", "provide(name, content = nil, &block)", "provide :${1:title}, \"${2}\"", "ActionView::Helpers::CaptureHelper"),
    h!("content_tag", "content_tag(name, content_or_options_with_block = nil, options = nil, escape = true, &block)", "content_tag :${1:div}, ${2:content}, class: \"${3}\"", "ActionView::Helpers::TagHelper"),
    h!("tag", "tag.<name>(content = nil, **options, &block)", "tag.${1:div}(${2:content}, class: \"${3}\")", "ActionView::Helpers::TagHelper"),
    h!("javascript_include_tag", "javascript_include_tag(*sources)", "javascript_include_tag \"${1:application}\"", "ActionView::Helpers::AssetTagHelper"),
    h!("stylesheet_link_tag", "stylesheet_link_tag(*sources)", "stylesheet_link_tag \"${1:application}\", \"data-turbo-track\": \"reload\"", "ActionView::Helpers::AssetTagHelper"),
    h!("javascript_importmap_tags", "javascript_importmap_tags(entry_point = \"application\")", "javascript_importmap_tags"),
    h!("csrf_meta_tags", "csrf_meta_tags", "csrf_meta_tags", "ActionView::Helpers::CsrfHelper"),
    h!("csp_meta_tag", "csp_meta_tag", "csp_meta_tag", "ActionView::Helpers::CspHelper"),
    h!("turbo_frame_tag", "turbo_frame_tag(*ids, src: nil, target: nil, **attributes, &block)", "turbo_frame_tag ${1:dom_id(@record)} do", "Turbo::FramesHelper"),
    h!("turbo_stream_from", "turbo_stream_from(*streamables, **attributes)", "turbo_stream_from ${1:@record}", "Turbo::StreamsHelper"),
    h!("turbo_stream", "turbo_stream.<action>(target, content = nil, &block)", "turbo_stream.${1|append,prepend,replace,update,remove,before,after|} ${2:target}", "Turbo::StreamsHelper"),
    h!("dom_id", "dom_id(record, prefix = nil)", "dom_id(${1:@record})", "ActionView::RecordIdentifier"),
    h!("dom_class", "dom_class(record_or_class, prefix = nil)", "dom_class(${1:@record})", "ActionView::RecordIdentifier"),
    h!("t", "t(key, **options) — translate", "t(\".${1:key}\")", "ActionView::Helpers::TranslationHelper"),
    h!("translate", "translate(key, **options)", "translate(\".${1:key}\")", "ActionView::Helpers::TranslationHelper"),
    h!("l", "l(object, **options) — localize", "l(${1:date}, format: :${2:long})", "ActionView::Helpers::TranslationHelper"),
    h!("localize", "localize(object, **options)", "localize(${1:date})", "ActionView::Helpers::TranslationHelper"),
    h!("number_to_currency", "number_to_currency(number, options = {})", "number_to_currency(${1:amount})", "ActionView::Helpers::NumberHelper"),
    h!("number_with_delimiter", "number_with_delimiter(number, options = {})", "number_with_delimiter(${1:number})", "ActionView::Helpers::NumberHelper"),
    h!("number_to_percentage", "number_to_percentage(number, options = {})", "number_to_percentage(${1:number}, precision: ${2:0})", "ActionView::Helpers::NumberHelper"),
    h!("number_to_human_size", "number_to_human_size(number, options = {})", "number_to_human_size(${1:bytes})", "ActionView::Helpers::NumberHelper"),
    h!("number_to_human", "number_to_human(number, options = {})", "number_to_human(${1:number})", "ActionView::Helpers::NumberHelper"),
    h!("pluralize", "pluralize(count, singular, plural = nil)", "pluralize(${1:count}, \"${2:item}\")", "ActionView::Helpers::TextHelper"),
    h!("truncate", "truncate(text, options = {})", "truncate(${1:text}, length: ${2:30})", "ActionView::Helpers::TextHelper"),
    h!("simple_format", "simple_format(text, html_options = {}, options = {})", "simple_format(${1:text})", "ActionView::Helpers::TextHelper"),
    h!("sanitize", "sanitize(html, options = {})", "sanitize(${1:html})", "ActionView::Helpers::SanitizeHelper"),
    h!("strip_tags", "strip_tags(html)", "strip_tags(${1:html})", "ActionView::Helpers::SanitizeHelper"),
    h!("highlight", "highlight(text, phrases, options = {})", "highlight(${1:text}, ${2:phrase})", "ActionView::Helpers::TextHelper"),
    h!("excerpt", "excerpt(text, phrase, options = {})", "excerpt(${1:text}, ${2:phrase})", "ActionView::Helpers::TextHelper"),
    h!("time_ago_in_words", "time_ago_in_words(from_time)", "time_ago_in_words(${1:time})", "ActionView::Helpers::DateHelper"),
    h!("distance_of_time_in_words", "distance_of_time_in_words(from_time, to_time = 0)", "distance_of_time_in_words(${1:from}, ${2:to})", "ActionView::Helpers::DateHelper"),
    h!("time_tag", "time_tag(date_or_time, *args, &block)", "time_tag ${1:time}", "ActionView::Helpers::DateHelper"),
    h!("raw", "raw(stringish)", "raw(${1:html})", "ActionView::Helpers::OutputSafetyHelper"),
    h!("html_safe", "String#html_safe", "html_safe"),
    h!("safe_join", "safe_join(array, sep = $,)", "safe_join(${1:array}, \"${2:, }\")", "ActionView::Helpers::OutputSafetyHelper"),
    h!("capture", "capture(*args, &block)", "capture do", "ActionView::Helpers::CaptureHelper"),
    h!("cache", "cache(name = {}, options = {}, &block)", "cache ${1:@record} do", "ActionView::Helpers::CacheHelper"),
    h!("cache_if", "cache_if(condition, name = {}, options = {}, &block)", "cache_if ${1:cond}, ${2:@record} do", "ActionView::Helpers::CacheHelper"),
    h!("current_page?", "current_page?(options)", "current_page?(${1:path})", "ActionView::Helpers::UrlHelper"),
    h!("url_for", "url_for(options = nil)", "url_for(${1:options})", "ActionView::RoutingUrlFor"),
    h!("asset_path", "asset_path(source, options = {})", "asset_path(\"${1:file}\")", "ActionView::Helpers::AssetUrlHelper"),
    h!("image_path", "image_path(source, options = {})", "image_path(\"${1:file.png}\")", "ActionView::Helpers::AssetUrlHelper"),
    h!("favicon_link_tag", "favicon_link_tag(source = 'favicon.ico', options = {})", "favicon_link_tag", "ActionView::Helpers::AssetTagHelper"),
    h!("hidden_field_tag", "hidden_field_tag(name, value = nil, options = {})", "hidden_field_tag :${1:name}, ${2:value}", "ActionView::Helpers::FormTagHelper"),
    h!("text_field_tag", "text_field_tag(name, value = nil, options = {})", "text_field_tag :${1:name}", "ActionView::Helpers::FormTagHelper"),
    h!("select_tag", "select_tag(name, option_tags = nil, options = {})", "select_tag :${1:name}, options_for_select(${2:choices})", "ActionView::Helpers::FormTagHelper"),
    h!("options_for_select", "options_for_select(container, selected = nil)", "options_for_select(${1:choices}, ${2:selected})", "ActionView::Helpers::FormOptionsHelper"),
    h!("options_from_collection_for_select", "options_from_collection_for_select(collection, value_method, text_method, selected = nil)", "options_from_collection_for_select(${1:collection}, :id, :${2:name})", "ActionView::Helpers::FormOptionsHelper"),
    h!("label_tag", "label_tag(name = nil, content_or_options = nil, options = nil, &block)", "label_tag :${1:name}, \"${2:Label}\"", "ActionView::Helpers::FormTagHelper"),
    h!("submit_tag", "submit_tag(value = 'Save changes', options = {})", "submit_tag \"${1:Save}\"", "ActionView::Helpers::FormTagHelper"),
    h!("button_tag", "button_tag(content_or_options = nil, options = nil, &block)", "button_tag \"${1:text}\", type: \"${2:submit}\"", "ActionView::Helpers::FormTagHelper"),
    h!("check_box_tag", "check_box_tag(name, value = '1', checked = false, options = {})", "check_box_tag :${1:name}", "ActionView::Helpers::FormTagHelper"),
    h!("radio_button_tag", "radio_button_tag(name, value, checked = false, options = {})", "radio_button_tag :${1:name}, ${2:value}", "ActionView::Helpers::FormTagHelper"),
    h!("text_area_tag", "text_area_tag(name, content = nil, options = {})", "text_area_tag :${1:name}", "ActionView::Helpers::FormTagHelper"),
    h!("file_field_tag", "file_field_tag(name, options = {})", "file_field_tag :${1:name}", "ActionView::Helpers::FormTagHelper"),
    h!("date_select", "date_select(object_name, method, options = {}, html_options = {})", "date_select :${1:object}, :${2:attr}", "ActionView::Helpers::DateHelper"),
    h!("flash", "flash — ActionDispatch::Flash::FlashHash", "flash"),
    h!("params", "params — ActionController::Parameters", "params"),
    h!("request", "request — ActionDispatch::Request", "request"),
    h!("controller_name", "controller_name", "controller_name"),
    h!("action_name", "action_name", "action_name"),
    h!("debug", "debug(object)", "debug(${1:object})", "ActionView::Helpers::DebugHelper"),
    h!("notice", "notice — flash[:notice]", "notice"),
    h!("alert", "alert — flash[:alert]", "alert"),
    h!("local_assigns", "local_assigns — partial locals hash", "local_assigns"),
    h!("protect_against_forgery?", "protect_against_forgery?", "protect_against_forgery?"),
    h!("csrf_token", "form_authenticity_token", "form_authenticity_token"),
];

/// Methods available on a form builder (`f.`).
pub const FORM_BUILDER: &[&str] = &[
    "label", "text_field", "email_field", "password_field", "number_field", "telephone_field", "phone_field", "url_field",
    "search_field", "date_field", "datetime_field", "datetime_local_field", "time_field", "month_field", "week_field",
    "color_field", "range_field", "hidden_field", "file_field", "text_area", "check_box", "checkbox", "radio_button",
    "select", "collection_select", "collection_check_boxes", "collection_radio_buttons", "grouped_collection_select",
    "time_zone_select", "date_select", "time_select", "datetime_select", "rich_text_area", "submit", "button",
    "fields_for", "fields", "object", "object_name", "id", "field_id", "field_name",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn url(name: &str) -> Option<String> { HELPERS.iter().find(|h| h.name == name)?.doc_url() }

    #[test]
    fn doc_urls() {
        assert_eq!(url("link_to").unwrap(), "https://api.rubyonrails.org/classes/ActionView/Helpers/UrlHelper.html#method-i-link_to");
        assert_eq!(url("content_for?").unwrap(), "https://api.rubyonrails.org/classes/ActionView/Helpers/CaptureHelper.html#method-i-content_for-3F");
        assert_eq!(url("turbo_frame_tag").unwrap(), "https://www.rubydoc.info/gems/turbo-rails/Turbo/FramesHelper#turbo_frame_tag-instance_method");
        assert_eq!(url("params"), None);
    }
}
