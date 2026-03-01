((tag_name) @keyword
  (#any-of? @keyword
    "@param" "@return" "@returns" "@throws" "@exception"
    "@see" "@since" "@version" "@author" "@deprecated"
    "@serial" "@serialField" "@serialData"
    "@link" "@linkplain" "@value" "@literal" "@code"
    "@inheritDoc" "@docRoot" "@hidden" "@index"
    "@provides" "@uses" "@implSpec" "@implNote" "@apiNote")
  (#set! "priority" 105))

((tag
  (tag_name) @_param
  (identifier) @variable.parameter)
  (#any-of? @_param "@param"))

(function (identifier) @function)

(function_link) @function

[
  "<a"
  ">"
  "</a>"
] @tag

[
  "@code"
  "@endcode"
] @keyword

(code_block_language) @label

["(" ")" "{" "}" "[" "]"] @punctuation.bracket
