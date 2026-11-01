import { Controller } from "@hotwired/stimulus";

export default class extends Controller {
  static targets = ["query", "results"];
  static values = {
    url: String,
    // wait: milliseconds before a request
    debounceWait: { type: Number, default: 300 },
  };
  static classes = ["loading"];
  static outlets = ["result-list"];

  search() {
    this.element.classList.add(this.loadingClass);
  }
}
