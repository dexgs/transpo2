const languageSelectForm = document.getElementById("language-select-form");
const languageSelect = document.getElementById("language-select");

languageSelect.onchange = function() { languageSelectForm.submit() };
